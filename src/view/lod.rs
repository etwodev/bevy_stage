//! Level of detail, authored as sibling nodes in the glTF.
//!
//! An artist exports `Rock_LOD0`, `Rock_LOD1`, `Rock_LOD2` and the plugin wires
//! them to Bevy's [`VisibilityRange`], which evaluates on the GPU. Nothing has
//! to be declared in code: the naming *is* the declaration.

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;

/// Distances at which each level of detail gives way to the next.
#[derive(Resource, Debug, Clone, Reflect)]
pub struct LodConfig {
    /// The distance at which LOD *n* hands over to LOD *n+1*.
    ///
    /// A model with more levels than there are thresholds keeps using the last
    /// one, spaced by [`LodConfig::falloff`].
    pub distances: Vec<f32>,
    /// The width of the band over which two levels crossfade.
    ///
    /// Bevy dithers the crossfade, so the two meshes must sit in exactly the
    /// same place — which is what exporting them as siblings gives you. Set to
    /// zero for a hard switch.
    pub crossfade: f32,
    /// How far apart to space thresholds beyond the end of `distances`.
    pub falloff: f32,
    /// Measure from the mesh's bounding box centre rather than its origin.
    ///
    /// Usually what you want: a large object whose origin sits at one corner
    /// otherwise switches detail based on that corner.
    pub use_aabb: bool,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            distances: vec![25.0, 60.0, 140.0],
            crossfade: 6.0,
            falloff: 120.0,
            use_aabb: true,
        }
    }
}

impl LodConfig {
    /// The distance at which level `index` hands over to the next.
    fn threshold(&self, index: usize) -> f32 {
        match self.distances.get(index) {
            Some(distance) => *distance,
            None => {
                let last = self.distances.last().copied().unwrap_or(0.0);
                let extra = (index + 1 - self.distances.len()) as f32;
                last + self.falloff * extra
            }
        }
    }
}

/// Build the visibility range for each level of a LOD group.
///
/// Level *n*'s fade-out band is exactly level *n+1*'s fade-in band, so one
/// takes over precisely as the other lets go and the object is never missing
/// or doubled.
pub fn lod_ranges(levels: usize, config: &LodConfig) -> Vec<VisibilityRange> {
    let half = config.crossfade.max(0.0) / 2.0;
    let mut ranges = Vec::with_capacity(levels);

    for level in 0..levels {
        let is_last = level + 1 == levels;

        // The near edge: zero for LOD0, otherwise where the previous level
        // finished handing over.
        let start = if level == 0 {
            0.0
        } else {
            config.threshold(level - 1)
        };
        // The far edge: the last level never fades out, so it stays visible to
        // the horizon rather than leaving a hole.
        let end = if is_last {
            f32::MAX
        } else {
            config.threshold(level)
        };

        let start_margin = if level == 0 {
            0.0..0.0
        } else {
            (start - half).max(0.0)..(start + half)
        };
        let end_margin = if is_last {
            f32::MAX..f32::MAX
        } else {
            (end - half).max(start_margin.end)..(end + half).max(start_margin.end)
        };

        ranges.push(VisibilityRange {
            start_margin,
            end_margin,
            use_aabb: config.use_aabb,
        });
    }
    ranges
}

/// Split a node name into its base and level of detail.
///
/// Accepts `Rock_LOD1`, `Rock.lod1` and `Rock-LOD1`, because which separator a
/// pipeline uses is a coin flip.
pub fn parse_lod_name(name: &str) -> Option<(&str, usize)> {
    let lower = name.to_ascii_lowercase();
    let marker = lower.rfind("lod")?;
    if marker == 0 {
        return None;
    }

    let level: usize = name[marker + 3..].parse().ok()?;
    let base = name[..marker].trim_end_matches(['_', '.', '-', ' ']);
    if base.is_empty() {
        return None;
    }
    Some((base, level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_names_parse_whatever_separator_the_pipeline_used() {
        assert_eq!(parse_lod_name("Rock_LOD0"), Some(("Rock", 0)));
        assert_eq!(parse_lod_name("Rock.lod2"), Some(("Rock", 2)));
        assert_eq!(parse_lod_name("Wall-LOD11"), Some(("Wall", 11)));
        // Not LOD markers.
        assert_eq!(parse_lod_name("Rock"), None);
        assert_eq!(parse_lod_name("LOD0"), None);
        assert_eq!(parse_lod_name("Rock_LODx"), None);
    }

    #[test]
    fn each_level_takes_over_exactly_where_the_last_one_lets_go() {
        let config = LodConfig {
            distances: vec![20.0, 50.0],
            crossfade: 4.0,
            falloff: 100.0,
            use_aabb: true,
        };
        let ranges = lod_ranges(3, &config);

        // LOD0 fades out over 18..22; LOD1 fades in over exactly the same band,
        // so the two cross over with no gap and no double-draw.
        assert_eq!(ranges[0].end_margin, 18.0..22.0);
        assert_eq!(ranges[1].start_margin, 18.0..22.0);
        assert_eq!(ranges[1].end_margin, 48.0..52.0);
        assert_eq!(ranges[2].start_margin, 48.0..52.0);
    }

    #[test]
    fn the_nearest_level_is_visible_from_zero_and_the_last_never_disappears() {
        let ranges = lod_ranges(3, &LodConfig::default());

        assert_eq!(ranges[0].start_margin, 0.0..0.0, "LOD0 must be visible up close");
        assert_eq!(
            ranges[2].end_margin.start,
            f32::MAX,
            "the coarsest level must not leave a hole in the distance"
        );
    }

    #[test]
    fn a_single_level_is_always_visible() {
        let ranges = lod_ranges(1, &LodConfig::default());
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start_margin, 0.0..0.0);
        assert_eq!(ranges[0].end_margin.start, f32::MAX);
    }

    #[test]
    fn more_levels_than_thresholds_keep_getting_further_apart() {
        let config = LodConfig {
            distances: vec![10.0],
            crossfade: 0.0,
            falloff: 50.0,
            use_aabb: false,
        };
        let ranges = lod_ranges(4, &config);

        // Thresholds continue at 10, 60, 110 rather than collapsing onto each
        // other, which would make the extra levels useless.
        assert_eq!(ranges[0].end_margin.start, 10.0);
        assert_eq!(ranges[1].end_margin.start, 60.0);
        assert_eq!(ranges[2].end_margin.start, 110.0);
        assert_eq!(ranges[3].end_margin.start, f32::MAX);
    }

    #[test]
    fn ranges_never_violate_bevys_ordering_invariant() {
        // VisibilityRange requires start_margin.end <= end_margin.start. A wide
        // crossfade against close thresholds is the case that would break it.
        let config = LodConfig {
            distances: vec![5.0, 7.0],
            crossfade: 20.0,
            falloff: 30.0,
            use_aabb: true,
        };
        for range in lod_ranges(3, &config) {
            assert!(
                range.start_margin.end <= range.end_margin.start,
                "invalid range: start_margin {:?} overlaps end_margin {:?}",
                range.start_margin,
                range.end_margin
            );
        }
    }
}
