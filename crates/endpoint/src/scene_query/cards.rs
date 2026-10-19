//! The kind cards of `scene.query`: for each question kind the fields it needs
//! and one working call. The schema text, the `missing_arg` refusal and the
//! `api.describe` entries all read this one table. Split out of the parent file
//! to keep it under the size ratchet.

use super::QueryKind;

/// One question kind as the door teaches it: the fields it needs (`a|b`
/// means one of them) and a working call. The schema text, the `missing_arg`
/// refusal and the `api.describe` entries all read this one table.
pub struct KindCard {
    /// The kind.
    pub kind: QueryKind,
    /// Required fields, comma separated; `-` when none.
    pub needs: &'static str,
    /// A complete working `scene.query` argument object for this kind.
    pub example: &'static str,
}

/// One card per kind, in discriminant order.
pub const KIND_CARDS: [KindCard; 24] = [
    KindCard {
        kind: QueryKind::Near,
        needs: "anchor",
        example: r#"{"kind":"near","anchor":"lamp_1 [<uuid>]","radius_cm":300}"#,
    },
    KindCard {
        kind: QueryKind::Between,
        needs: "anchor, other",
        example: r#"{"kind":"between","anchor":"lamp_1 [<uuid>]","other":"crate_1 [<uuid>]"}"#,
    },
    KindCard {
        kind: QueryKind::Visible,
        needs: "eye_m",
        example: r#"{"kind":"visible","eye_m":[0.0,1.6,0.0],"yaw":0.0}"#,
    },
    KindCard {
        kind: QueryKind::Reachable,
        needs: "anchor",
        example: r#"{"kind":"reachable","anchor":"lamp_1 [<uuid>]"}"#,
    },
    KindCard {
        kind: QueryKind::CoverNear,
        needs: "anchor, eye_m",
        example: r#"{"kind":"cover_near","anchor":"lamp_1 [<uuid>]","eye_m":[0.0,1.6,0.0],"radius_cm":500}"#,
    },
    KindCard {
        kind: QueryKind::LargestOpen,
        needs: "-",
        example: r#"{"kind":"largest_open","capsule_radius_cm":30}"#,
    },
    KindCard {
        kind: QueryKind::FreeSpace,
        needs: "-",
        example: r#"{"kind":"free_space","max_samples":4,"min_gap_cm":100}"#,
    },
    KindCard {
        kind: QueryKind::Cone,
        needs: "half_angle_deg, anchor|eye_m",
        example: r#"{"kind":"cone","anchor":"lamp_1 [<uuid>]","half_angle_deg":45,"radius_cm":500}"#,
    },
    KindCard {
        kind: QueryKind::Relation,
        needs: "anchor, relation",
        example: r#"{"kind":"relation","anchor":"lamp_1 [<uuid>]","relation":"left_of"}"#,
    },
    KindCard {
        kind: QueryKind::Sees,
        needs: "anchor, other",
        example: r#"{"kind":"sees","anchor":"lamp_1 [<uuid>]","other":"crate_1 [<uuid>]"}"#,
    },
    KindCard {
        kind: QueryKind::WhyHidden,
        needs: "anchor, eye_m",
        example: r#"{"kind":"why_hidden","anchor":"crate_1 [<uuid>]","eye_m":[0.0,1.6,0.0],"yaw":0.0}"#,
    },
    KindCard {
        kind: QueryKind::Clearance,
        needs: "anchor",
        example: r#"{"kind":"clearance","anchor":"crate_1 [<uuid>]","radius_cm":80}"#,
    },
    KindCard {
        kind: QueryKind::Slide,
        needs: "anchor, dir",
        example: r#"{"kind":"slide","anchor":"crate_1 [<uuid>]","dir":"+x","radius_cm":500}"#,
    },
    KindCard {
        kind: QueryKind::Ground,
        needs: "eye_m",
        example: r#"{"kind":"ground","eye_m":[2.0,3.0,1.0]}"#,
    },
    KindCard {
        kind: QueryKind::Measure,
        needs: "anchor, other",
        example: r#"{"kind":"measure","anchor":"crate_1 [<uuid>]","other":"wall_1 [<uuid>]","what":"gap","radius_cm":90}"#,
    },
    KindCard {
        kind: QueryKind::RoomOf,
        needs: "anchor",
        example: r#"{"kind":"room_of","anchor":"crate_1 [<uuid>]"}"#,
    },
    KindCard {
        kind: QueryKind::InRoom,
        needs: "room",
        example: r#"{"kind":"in_room","room":1}"#,
    },
    KindCard {
        kind: QueryKind::ExitsOf,
        needs: "anchor|room",
        example: r#"{"kind":"exits_of","room":1}"#,
    },
    KindCard {
        kind: QueryKind::Glance,
        needs: "eye_m",
        example: r#"{"kind":"glance","eye_m":[0.0,1.6,0.0],"yaw":0.0,"half_angle_deg":45}"#,
    },
    KindCard {
        kind: QueryKind::Coverage,
        needs: "anchor, eye_m",
        example: r#"{"kind":"coverage","anchor":"crate_1 [<uuid>]","eye_m":[0.0,1.6,0.0],"yaw":0.0,"pitch":0.0,"half_angle_deg":30}"#,
    },
    KindCard {
        kind: QueryKind::View,
        needs: "eye_m",
        example: r#"{"kind":"view","eye_m":[0.0,1.6,0.0],"yaw":0.0,"cells":[24,16,0,0]}"#,
    },
    KindCard {
        kind: QueryKind::Lidar,
        needs: "eye_m",
        example: r#"{"kind":"lidar","eye_m":[0.0,1.6,0.0],"yaw":0.0}"#,
    },
    KindCard {
        kind: QueryKind::Map,
        needs: "eye_m",
        example: r#"{"kind":"map","eye_m":[0.0,1.6,0.0],"radius_cm":600}"#,
    },
    KindCard {
        kind: QueryKind::Pixel,
        needs: "eye_m",
        example: r#"{"kind":"pixel","eye_m":[0.0,1.6,0.0],"yaw":0.0,"cells":[24,16,12,8]}"#,
    },
];

impl KindCard {
    /// The card for `kind`.
    pub fn of(kind: QueryKind) -> &'static KindCard {
        KIND_CARDS
            .iter()
            .find(|card| card.kind == kind)
            .unwrap_or(&KIND_CARDS[0])
    }
}
