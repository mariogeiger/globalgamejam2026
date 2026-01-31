use serde::{Deserialize, Serialize};

use crate::glb::{SPAWNS_TEAM_A, SPAWNS_TEAM_B};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Team {
    A,
    B,
}

impl Team {
    pub fn color(&self) -> [f32; 4] {
        match self {
            Team::A => [0.2, 0.4, 1.0, 1.0],
            Team::B => [1.0, 0.3, 0.2, 1.0],
        }
    }

    pub fn spawn_points(&self) -> &'static [[f32; 3]] {
        match self {
            Team::A => SPAWNS_TEAM_A,
            Team::B => SPAWNS_TEAM_B,
        }
    }
}
