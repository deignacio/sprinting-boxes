use crate::scoring::EndZoneOccupancy;

/// Which end zone emptied first after a cliff, indicating the pulling team.
#[derive(Debug, Clone, PartialEq)]
pub enum PullSide {
    Left,
    Right,
    /// Both emptied simultaneously; tiebreaker resolved by earlier asymmetry.
    Tie,
    /// Neither end zone reached zero — likely a false positive cliff.
    Unknown,
}

/// Determine which team pulled by finding which end zone emptied first.
///
/// `history` is a slice of `(frame_index, EndZoneOccupancy)` in ascending frame order,
/// covering the window around the cliff (lookback + lookahead).
/// `debounce_frames` is the number of consecutive zero-count frames required before
/// declaring an end zone empty (default 2).
pub fn detect_pull_side(
    history: &[(usize, EndZoneOccupancy)],
    debounce_frames: usize,
) -> PullSide {
    let debounce = debounce_frames.max(1);

    let mut left_zero_run = 0usize;
    let mut right_zero_run = 0usize;
    let mut left_emptied_at: Option<usize> = None;
    let mut right_emptied_at: Option<usize> = None;

    for (frame_idx, occ) in history {
        if occ.left == 0.0 {
            left_zero_run += 1;
            if left_zero_run >= debounce && left_emptied_at.is_none() {
                left_emptied_at = Some(*frame_idx);
            }
        } else {
            left_zero_run = 0;
        }

        if occ.right == 0.0 {
            right_zero_run += 1;
            if right_zero_run >= debounce && right_emptied_at.is_none() {
                right_emptied_at = Some(*frame_idx);
            }
        } else {
            right_zero_run = 0;
        }
    }

    match (left_emptied_at, right_emptied_at) {
        (Some(l), Some(r)) => {
            if l < r {
                PullSide::Left
            } else if r < l {
                PullSide::Right
            } else {
                // Simultaneous: scan backward for the first asymmetry as tiebreaker
                let tiebreak = history
                    .iter()
                    .rev()
                    .find(|(fi, occ)| *fi < l && occ.left != occ.right)
                    .map(|(_, occ)| {
                        if occ.left < occ.right {
                            PullSide::Left
                        } else {
                            PullSide::Right
                        }
                    });
                tiebreak.unwrap_or(PullSide::Tie)
            }
        }
        (Some(_), None) => PullSide::Left,
        (None, Some(_)) => PullSide::Right,
        (None, None) => PullSide::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(left: f32, right: f32) -> EndZoneOccupancy {
        EndZoneOccupancy { left, right, field: 0.0 }
    }

    fn history(pairs: &[(usize, f32, f32)]) -> Vec<(usize, EndZoneOccupancy)> {
        pairs.iter().map(|(i, l, r)| (*i, occ(*l, *r))).collect()
    }

    #[test]
    fn left_empties_first() {
        let h = history(&[
            (0, 0.5, 0.5),
            (1, 0.0, 0.5),
            (2, 0.0, 0.0), // left confirmed empty at frame 1 (2 consecutive), right at frame 2
            (3, 0.0, 0.0),
        ]);
        assert_eq!(detect_pull_side(&h, 2), PullSide::Left);
    }

    #[test]
    fn right_empties_first() {
        let h = history(&[
            (0, 0.5, 0.5),
            (1, 0.5, 0.0),
            (2, 0.0, 0.0),
            (3, 0.0, 0.0),
        ]);
        assert_eq!(detect_pull_side(&h, 2), PullSide::Right);
    }

    #[test]
    fn simultaneous_with_tiebreaker() {
        // Both empty at frame 2; earlier asymmetry at frame 0 favors left
        let h = history(&[
            (0, 0.3, 0.5), // left < right → tiebreaker says Left
            (1, 0.0, 0.0),
            (2, 0.0, 0.0),
        ]);
        assert_eq!(detect_pull_side(&h, 2), PullSide::Left);
    }

    #[test]
    fn unknown_when_neither_empties() {
        let h = history(&[
            (0, 0.5, 0.5),
            (1, 0.3, 0.4),
            (2, 0.2, 0.3),
        ]);
        assert_eq!(detect_pull_side(&h, 2), PullSide::Unknown);
    }
}
