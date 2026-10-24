//! BG1: the `ServerHandler` surface of [`McpServer`], split from `mcp.rs` so
//! the session core stays readable.
//!
//! This module owns the handshake, the tool, prompt and resource lists, the
//! capability advertisement and the live push channel. The session state and
//! the command path stay in `mcp.rs`; this layer only wires them to rmcp.

use std::borrow::Cow;

use engine_editor::save::save_to_bytes;
use engine_scene::ids::content_digest;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, GetPromptRequestParams, GetPromptResponse,
    GetPromptResult, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, PromptMessage, PromptsCapability,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    ResourceUpdatedNotification, ResourceUpdatedNotificationParam, ResourcesCapability, ResultType,
    Role, ServerCapabilities, ServerConfig, ServerNotification, SubscribeRequestParams,
    SubscriptionFilter, ToolAnnotations, ToolListChangedNotification, ToolsCapability,
    UnsubscribeRequestParams,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::packs::PackRegistry;

use super::{McpServer, ServerState, SUPPORTED_PROTOCOL_VERSIONS};

impl ServerState {
    /// BG1: the live scene view the `e2040://scene` resource serves and the
    /// push channel watches.
    ///
    /// The digest is the content digest `scene.save` prints, so the same
    /// document always renders the same bytes and any landed write moves them.
    pub fn scene_view(&self) -> String {
        let digest = content_digest(&save_to_bytes(&self.doc));
        crate::resources::scene_text(self.doc.revision, self.doc.hierarchy.len(), &digest)
    }
}

impl McpServer {
    /// BG1: push the change notes a call queued before the answer leaves.
    ///
    /// One note per change per session. A client with a live
    /// `subscriptions/listen` stream gets the change on its stream (filter
    /// checked, subscription-attributed); a legacy client without one gets a
    /// tool-list change on the session itself. A 2026-07-28 session hears
    /// changes only on the streams it opened, never on the bare session. The
    /// scene view is recomputed only while a scene subscriber is live, so a
    /// read-only call costs no digest.
    pub(super) async fn flush_notices(&self, context: &RequestContext<RoleServer>) {
        // rmcp reports no version for a legacy handshake done via `initialize`.
        let modern_session = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        let (list_changed, list_sinks, scene_sinks, scene_legacy, scene_moved) = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            // A reborrow keeps `packs_pushed` and `loaded_packs` disjoint fields.
            let state = &mut *state;
            let list_changed = state.packs_pushed != state.loaded_packs;
            if list_changed {
                state.packs_pushed.clone_from(&state.loaded_packs);
            }
            let watching = state.scene_subscribed || !state.scene_subscribers.is_empty();
            let scene_moved = watching && state.scene_pushed.is_some() && {
                let view = state.scene_view();
                let moved = state.scene_pushed.as_deref() != Some(view.as_str());
                if moved {
                    state.scene_pushed = Some(view);
                }
                moved
            };
            (
                list_changed,
                state.list_subscribers.clone(),
                state.scene_subscribers.clone(),
                state.scene_subscribed,
                scene_moved,
            )
        };
        if list_changed {
            let note = ServerNotification::ToolListChangedNotification(
                ToolListChangedNotification::default(),
            );
            if list_sinks.is_empty() {
                if !modern_session {
                    let _ = context.peer.send_notification(note).await;
                }
            } else {
                for sink in &list_sinks {
                    let _ = sink.send(note.clone()).await;
                }
            }
        }
        if scene_moved {
            let note =
                ServerNotification::ResourceUpdatedNotification(ResourceUpdatedNotification::new(
                    ResourceUpdatedNotificationParam::new(crate::resources::SCENE_URI),
                ));
            for sink in &scene_sinks {
                let _ = sink.send(note.clone()).await;
            }
            if scene_legacy {
                let _ = context.peer.send_notification(note).await;
            }
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::default();
        // BG1: the tool list moves with `api.load`, so the door announces the
        // change instead of making the client poll it.
        let mut tools = ToolsCapability::default();
        tools.list_changed = Some(true);
        capabilities.tools = Some(tools);
        // D16: the door serves four static resources (crate map, door
        // reference, error catalog, game.toml schema) plus the live scene
        // view. Resources cost no tool-list budget: a read side, not verbs.
        // BG1: a client may subscribe to the live scene view.
        let mut resources = ResourcesCapability::default();
        resources.subscribe = Some(true);
        capabilities.resources = Some(resources);
        // The door always publishes prompts: the shipped skills plus the five
        // D16 scenarios, so the capability does not depend on the skills
        // registry being non-empty.
        capabilities.prompts = Some(PromptsCapability::default());
        let mut info = InitializeResult::new(capabilities);
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        "engine-endpoint".clone_into(&mut info.server_info.name);
        env!("CARGO_PKG_VERSION").clone_into(&mut info.server_info.version);
        info.instructions = Some(
            "Scene authoring door: scene.read for the revision, tx.preview for the diff \
             plus a single-use approval, tx.apply with that approval, tx.undo to revert. \
             More verbs live in packs: api.search \"pack\" lists them, api.load advertises \
             one, api.call runs any verb by name. No shell, code execution or file writes."
                .to_owned(),
        );
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.initialize_result(&request.protocol_version)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.call_by_name(request, context).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // The list moves with `api.load`, so 2026-07-28 clients get a zero
        // cache hint: re-list after loading a pack.
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: self.tools(),
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(CacheScope::Public),
        })
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult {
            prompts: self.prompt_list(),
            ..Default::default()
        })
    }

    /// D16: the four resources the door serves, in contract order.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut resources = crate::resources::resource_list();
        // BG1: the live scene view rides beside the four static families; its
        // body moves with the document, so it is read from the session.
        resources.push(crate::resources::scene_resource());
        Ok(ListResourcesResult {
            resources,
            ..Default::default()
        })
    }

    /// D16: one template per resource family, so a client can learn the
    /// child URI shape before it reads the root.
    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult {
            resource_templates: crate::resources::resource_templates(),
            ..Default::default()
        })
    }

    /// D16: read one resource body as text.
    ///
    /// The door reference row list is built from the live advertised tools,
    /// so the resource never drifts from the router.
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri.as_ref();
        if uri == crate::resources::SCENE_URI {
            // BG1: reading the live view fixes the push channel's baseline, so
            // only a write after this read announces another update.
            let text = match self.lock() {
                Ok(mut state) => {
                    let view = state.scene_view();
                    state.scene_pushed = Some(view.clone());
                    view
                }
                Err(error) => return Err(ErrorData::from(error)),
            };
            return Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
                vec![crate::resources::scene_contents(text)],
            )));
        }
        match crate::resources::read_resource(uri, &self.tool_reference_rows()) {
            Ok(contents) => Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
                vec![contents],
            ))),
            Err(error) => Err(ErrorData::invalid_params(error.to_string(), None)),
        }
    }

    /// BG1: the filter the door accepts on a `subscriptions/listen` stream.
    ///
    /// The door pushes exactly two kinds of change: the tool list (moved by
    /// `api.load`) and the live scene view. The SDK intersects this with what
    /// the client asked for and with what [`Self::get_info`] advertises, so a
    /// client is never acknowledged for a note the door does not send.
    fn accepted_subscription_filter(
        &self,
        _requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(
            SubscriptionFilter::builder()
                .tools_list_changed()
                .resource_subscription(crate::resources::SCENE_URI)
                .build(),
        )
    }

    /// BG1: run one established listen stream until the client cancels it.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let sink = context.sink().clone();
        let accepted = context.accepted();
        let wants_scene = accepted
            .resource_subscriptions
            .as_ref()
            .is_some_and(|uris| uris.iter().any(|uri| uri == crate::resources::SCENE_URI));
        let wants_list = accepted.tools_list_changed == Some(true);
        {
            let Ok(mut state) = self.state.lock() else {
                return Err(ErrorData::internal_error(
                    "server state lock poisoned by a failed call; restart the server",
                    None,
                ));
            };
            if wants_scene {
                // The subscriber starts from the view it can read right now.
                state.scene_pushed = Some(state.scene_view());
                state.scene_subscribers.push(sink.clone());
            }
            if wants_list {
                state.list_subscribers.push(sink.clone());
            }
        }
        context.cancelled().await;
        if let Ok(mut state) = self.state.lock() {
            state
                .scene_subscribers
                .retain(|live| live.id() != sink.id());
            state.list_subscribers.retain(|live| live.id() != sink.id());
        }
        Ok(())
    }

    /// BG1: legacy `resources/subscribe` (protocols before 2026-07-28, where
    /// no listen stream exists). Only the live scene view can be subscribed.
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if request.uri != crate::resources::SCENE_URI {
            return Err(ErrorData::invalid_params(
                format!(
                    "no subscription at `{}`; the door pushes `{}` when it moves",
                    request.uri,
                    crate::resources::SCENE_URI
                ),
                None,
            ));
        }
        let Ok(mut state) = self.state.lock() else {
            return Err(ErrorData::internal_error(
                "server state lock poisoned by a failed call; restart the server",
                None,
            ));
        };
        state.scene_pushed = Some(state.scene_view());
        state.scene_subscribed = true;
        Ok(())
    }

    /// BG1: legacy `resources/unsubscribe`.
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if request.uri == crate::resources::SCENE_URI {
            if let Ok(mut state) = self.state.lock() {
                state.scene_subscribed = false;
            }
        }
        Ok(())
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let Some(body) = self.prompt_text(&request.name) else {
            return Err(ErrorData::invalid_params("prompt not found", None));
        };
        Ok(GetPromptResponse::Complete(GetPromptResult::new(vec![
            PromptMessage::new_text(Role::User, body),
        ])))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned().or_else(|| {
            PackRegistry::shipped()
                .verb(name)
                .map(crate::packs::VerbSpec::model)
        })
    }
}

/// Annotations for read-only MCP tools: read-only on, open world off.
/// Owner of the pair every read-only tool's `annotations()` returns.
pub(crate) fn read_only_annotations() -> Option<ToolAnnotations> {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(true);
    annotations.open_world_hint = Some(false);
    Some(annotations)
}

/// Annotations for state-changing MCP tools: read-only off, open world off,
/// idempotent hint set only when a repeated call is a no-op.
pub(crate) fn write_annotations(idempotent: bool) -> Option<ToolAnnotations> {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(false);
    annotations.idempotent_hint = idempotent.then_some(true);
    annotations.open_world_hint = Some(false);
    Some(annotations)
}
