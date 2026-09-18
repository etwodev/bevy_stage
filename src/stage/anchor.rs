//! Placing one stage relative to another so their geometry lines up exactly.

use bevy::math::Affine3A;
use bevy::prelude::*;
use serde::Deserialize;

/// A named connection point in a level.
///
/// Authored as a node tagged `anchor` with an `id` property. A stage loaded as
/// a neighbour is positioned so that the anchor named by the incoming portal
/// lands exactly on that portal.
///
/// # Orientation convention
///
/// **An anchor points out of its own stage**, like a normal on the level
/// boundary: a door on the north wall faces north. Two connected anchors
/// therefore face each other, which is why [`AnchorAlignment::Facing`] is the
/// default.
///
/// This convention is worth following because it makes connections symmetric —
/// the same pair of anchors places B against A and A against B with identical
/// maths — so a player can walk back and forth through a doorway without the
/// world drifting. Anchors authored pointing *along* the direction of travel
/// instead need [`AnchorAlignment::Aligned`].
#[derive(Component, Debug, Clone, Default, Deserialize, Reflect)]
#[reflect(Component)]
pub struct StageAnchor {
    /// The anchor's name, unique within its stage.
    #[serde(default)]
    pub id: String,
}

/// How two connection points should be oriented with respect to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum AnchorAlignment {
    /// The two anchors face each other, as two sides of a doorway do.
    ///
    /// The default, because the overwhelmingly common case is walking *through*
    /// a connection: you exit A heading north and enter B heading north, which
    /// means B's entry anchor points back at you.
    #[default]
    Facing,
    /// The two anchors point the same way, for continuing a corridor or
    /// tiling terrain where both ends share a heading.
    Aligned,
}

impl AnchorAlignment {
    /// The rotation inserted between the two anchors.
    fn correction(self) -> Affine3A {
        match self {
            Self::Facing => Affine3A::from_rotation_y(core::f32::consts::PI),
            Self::Aligned => Affine3A::IDENTITY,
        }
    }
}

/// Strip scale from a transform, keeping only position and orientation.
///
/// Connection points are markers, not geometry. Artists routinely scale the
/// empty that marks a doorway so they can see it against the wall, and that
/// scale must not leak into the level being placed against it — it would
/// stretch the whole level, and compound through every further connection.
fn rigid(transform: &GlobalTransform) -> Affine3A {
    let (_, rotation, translation) = transform.to_scale_rotation_translation();
    Affine3A::from_rotation_translation(rotation, translation)
}

/// Compute the world transform for a stage root so that one of its anchors
/// lands on a given world-space connection point.
///
/// `anchor_local` is the anchor's transform *relative to its own stage root*,
/// which is what makes this composable: the stage can then be placed anywhere
/// without re-deriving anything.
///
/// Solves `root * anchor_local == target * correction` for `root`, ignoring the
/// scale of both connection points so the placed stage always comes out at
/// its authored size.
pub fn align_stage_to(
    target: &GlobalTransform,
    anchor_local: &GlobalTransform,
    alignment: AnchorAlignment,
) -> Transform {
    let placement = rigid(target) * alignment.correction() * rigid(anchor_local).inverse();
    Transform::from_matrix(Mat4::from(placement))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where an anchor ends up in the world once its stage is placed.
    fn placed(root: &Transform, anchor_local: &GlobalTransform) -> GlobalTransform {
        GlobalTransform::from(*root) * *anchor_local
    }

    fn approx(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn a_facing_connection_puts_the_anchors_in_the_same_place() {
        // A doorway in stage A, somewhere arbitrary and rotated.
        let target = GlobalTransform::from(
            Transform::from_xyz(10.0, 0.0, -20.0).with_rotation(Quat::from_rotation_y(0.7)),
        );
        // Stage B's entry anchor, offset from B's own origin.
        let anchor_local = GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 15.0));

        let root = align_stage_to(&target, &anchor_local, AnchorAlignment::Facing);
        let landed = placed(&root, &anchor_local);

        // The two connection points must coincide in space...
        assert!(
            approx(landed.translation(), target.translation()),
            "anchor landed at {:?}, expected {:?}",
            landed.translation(),
            target.translation()
        );
        // ...and face opposite ways, so walking through works.
        let dot = landed.forward().dot(*target.forward());
        assert!(dot < -0.999, "anchors should face each other, dot = {dot}");
    }

    #[test]
    fn an_aligned_connection_keeps_both_headings() {
        let target = GlobalTransform::from(
            Transform::from_xyz(-4.0, 2.0, 8.0).with_rotation(Quat::from_rotation_y(-1.2)),
        );
        let anchor_local = GlobalTransform::from(Transform::from_xyz(3.0, 0.0, 0.0));

        let root = align_stage_to(&target, &anchor_local, AnchorAlignment::Aligned);
        let landed = placed(&root, &anchor_local);

        assert!(approx(landed.translation(), target.translation()));
        let dot = landed.forward().dot(*target.forward());
        assert!(dot > 0.999, "anchors should share a heading, dot = {dot}");
    }

    #[test]
    fn placement_works_when_the_anchor_is_itself_rotated() {
        // An anchor authored at an angle inside its own stage is the case that
        // breaks naive "just subtract the offset" placement.
        let target = GlobalTransform::from(
            Transform::from_xyz(5.0, 1.0, 5.0).with_rotation(Quat::from_rotation_y(2.4)),
        );
        let anchor_local = GlobalTransform::from(
            Transform::from_xyz(2.0, 0.0, -7.0).with_rotation(Quat::from_rotation_y(-0.9)),
        );

        let root = align_stage_to(&target, &anchor_local, AnchorAlignment::Facing);
        let landed = placed(&root, &anchor_local);

        assert!(
            approx(landed.translation(), target.translation()),
            "landed at {:?}, expected {:?}",
            landed.translation(),
            target.translation()
        );
    }

    #[test]
    fn a_stage_anchored_at_its_own_origin_lands_on_the_target() {
        let target = GlobalTransform::from(Transform::from_xyz(3.0, 0.0, 0.0));
        let anchor_local = GlobalTransform::IDENTITY;

        let root = align_stage_to(&target, &anchor_local, AnchorAlignment::Aligned);

        // With the anchor at the stage origin and no correction, the stage root
        // simply moves to the target.
        assert!(approx(root.translation, Vec3::new(3.0, 0.0, 0.0)));
    }

    #[test]
    fn a_scaled_connection_point_does_not_stretch_the_level() {
        // Artists scale the empty marking a doorway so it is visible against
        // the wall. That must not scale the level placed against it — and if it
        // did, the error would compound through every further connection.
        let target = GlobalTransform::from(
            Transform::from_xyz(0.0, 1.5, -19.0).with_scale(Vec3::new(3.0, 3.0, 0.5)),
        );
        let anchor_local =
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 15.0).with_scale(Vec3::splat(2.0)));

        let root = align_stage_to(&target, &anchor_local, AnchorAlignment::Facing);

        let scale = root.scale;
        assert!(
            (scale - Vec3::ONE).length() < 1e-4,
            "placed stage came out scaled by {scale:?}"
        );
        // And it still lands in the right place.
        let landed = placed(&root, &GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 15.0)));
        assert!(approx(landed.translation(), target.translation()));
    }

    #[test]
    fn aligning_back_through_the_same_connection_recovers_the_original_placement() {
        // Stage A sits at the origin with a door partway along it.
        let a_root = Transform::IDENTITY;
        let a_door_local =
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -20.0).with_rotation(Quat::from_rotation_y(0.3)));
        let a_door_world = placed(&a_root, &a_door_local);

        // Place the hallway against that door.
        let hall_entry_local = GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 15.0));
        let hall_root = align_stage_to(&a_door_world, &hall_entry_local, AnchorAlignment::Facing);
        let hall_entry_world = placed(&hall_root, &hall_entry_local);

        // Now do the reverse: place stage A against the hallway's entry, using
        // A's own door as its anchor. It must land back where it started, or a
        // player walking out and back in would see the world shift.
        let recovered = align_stage_to(&hall_entry_world, &a_door_local, AnchorAlignment::Facing);

        assert!(
            approx(recovered.translation, a_root.translation),
            "recovered {:?}, expected {:?}",
            recovered.translation,
            a_root.translation
        );
        let drift = recovered.rotation.angle_between(a_root.rotation);
        assert!(drift < 1e-3, "rotation drifted by {drift} rad");
    }
}
