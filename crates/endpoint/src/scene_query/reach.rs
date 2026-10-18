//! AD20 reach honesty at the door: `scene.query reachable` answers from the
//! baked nav mesh a host handed the session, never from a guess.
//!
//! Consumer: [`super::ServerState::scene_query`]. Data owner: `engine-nav`
//! owns the mesh and the polygon adapter ([`engine_nav::reach::reach_view`]);
//! the host owns the bake. Without a mesh `reachable` refuses
//! `needs_nav_view`, the code `scene.spatial` uses for nav checks.

use engine_spatial::sight::ReachView;
use engine_spatial::Actor;

use crate::mcp::{McpToolError, ServerState};

impl ServerState {
    /// Hold the baked nav mesh of the staged floor. The host that bakes calls
    /// this before [`McpServer::with_state`](crate::mcp::McpServer::with_state);
    /// `scene.query reachable` then answers from the mesh polygons.
    #[must_use]
    pub fn with_nav(mut self, mesh: engine_nav::mesh::NavMesh) -> Self {
        self.nav = Some(mesh);
        self
    }
}

/// Nav polygon of every staged actor: the mesh gives each its own, an actor
/// off the walkable floor is left out and reads `blocked:off_mesh`.
pub(super) fn staged_reach(mesh: &engine_nav::mesh::NavMesh, actors: &[Actor]) -> ReachView {
    let rows: Vec<(u32, [f32; 3])> = actors
        .iter()
        .map(|actor| (actor.id, [actor.x, actor.y, actor.z]))
        .collect();
    engine_nav::reach::reach_view(mesh, &rows)
}

/// `reachable` with no nav mesh held: the refusal names the recovery.
pub(super) fn needs_nav_view() -> McpToolError {
    McpToolError::new(
        "needs_nav_view",
        "reachable needs a baked nav mesh and this session holds none; ask the host to bake \
         the floor, or use between, sees or near for questions that need no walkable floor",
    )
}
