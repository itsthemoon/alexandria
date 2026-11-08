use serde::Serialize;

mod format;
mod router;

pub use format::{
    class_is_link, class_speed_kmh, order_by_cell, push_geometry, Grid, Writer, C_FERRY, C_LIVING_STREET,
    C_MOTORWAY, C_MOTORWAY_LINK, C_OTHER, C_PRIMARY, C_PRIMARY_LINK, C_RESIDENTIAL, C_SECONDARY,
    C_SECONDARY_LINK, C_SERVICE, C_TERTIARY, C_TERTIARY_LINK, C_TRUNK, C_TRUNK_LINK, C_UNCLASSIFIED,
    F_CIRCULAR, F_FERRY, F_LINK, F_RESTRICTED, F_ROUNDABOUT,
};
pub use router::Roads;

#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub distance_m: f64,
    pub duration_s: f64,
    pub steps: Vec<Step>,
    pub line: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub text: String,
    pub distance_m: f64,
    pub at: [f64; 2],
}
