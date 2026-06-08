/// Normalized player counts per field region.
/// Each value is in [0.0, 1.0] where 1.0 = full team in that region.
#[derive(Debug, Clone, PartialEq)]
pub struct EndZoneOccupancy {
    pub left: f32,
    pub right: f32,
    pub field: f32,
}

/// Compute the pre-point score for a single frame.
///
/// Inputs are normalized counts (raw_count / team_size). Returns a score in [0, 1]
/// where higher means more likely a pre-point huddle state (both end zones occupied).
pub fn pre_point_score(occupancy: &EndZoneOccupancy, team_size: u32) -> f32 {
    let threshold = 2.0 / (team_size as f32);
    let min_ez_occupancy = occupancy.left.min(occupancy.right);

    let balance_term = if min_ez_occupancy >= threshold {
        min_ez_occupancy
    } else if min_ez_occupancy > 0.0 {
        min_ez_occupancy * 0.5
    } else {
        0.0
    };

    let ez_balance = (occupancy.left - occupancy.right).abs();
    let symmetry_bonus = (1.2 - ez_balance).clamp(0.0, 1.0);
    let field_term = (1.5 - occupancy.field).clamp(0.0, 1.0);

    let score = 2.0 * balance_term * symmetry_bonus * field_term;
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_occupied_equally() {
        let score = pre_point_score(&EndZoneOccupancy { left: 0.5, right: 0.5, field: 0.0 }, 7);
        assert!(score > 0.9, "score: {}", score);
    }

    #[test]
    fn empty_end_zones() {
        let score = pre_point_score(&EndZoneOccupancy { left: 0.0, right: 0.0, field: 0.0 }, 7);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn single_player_each_side() {
        // One player in each end zone: weak but non-zero signal
        let score = pre_point_score(&EndZoneOccupancy { left: 0.14, right: 0.14, field: 0.0 }, 7);
        assert!(score > 0.0 && score < 0.2, "score: {}", score);
    }

    #[test]
    fn single_player_one_side_only() {
        // Only one end zone occupied: score must be zero (both sides required)
        let score = pre_point_score(&EndZoneOccupancy { left: 0.14, right: 0.0, field: 0.0 }, 7);
        assert_eq!(score, 0.0, "score: {}", score);
    }
}
