//! An example of a "real-world" use case of bevy_lookup_curve in a bevy app.
//!
//! This example includes loading a curve asset, using it for some simple animation, and editing it with an in-app editor.
use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use bevy_lookup_curve::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(LookupCurvePlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, animate)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);

    // Load the curve asset
    const PATH: &str = "example.curve.ron";
    let curve_handle = asset_server.load(PATH);

    // Spawn a curve editor
    let editor_id = commands
        .spawn(LookupCurveEditor::with_save_path(
            curve_handle.clone(),
            format!("./assets/{PATH}"),
        ))
        .id();

    // Spawn a sprite that animates its x position using the curve
    commands.spawn((
        Sprite::from_image(asset_server.load("bevy_icon.png")),
        Transform::from_xyz(0., -200., 0.).with_scale(Vec3::splat(0.5)),
        AnimateX {
            curve: curve_handle.clone(),
            cache: LookupCache::new(),
            from: -400.0,
            to: 400.0,
            t: 0.0,
            dir: 1.0,
            speed: 0.3,
        },
        PreviewSample(editor_id),
    ));
}

/// Component that animates the x position of an entity using a [LookupCurve].
#[derive(Component)]
struct AnimateX {
    curve: Handle<LookupCurve>,
    cache: LookupCache,
    from: f32,
    to: f32,
    t: f32,
    dir: f32,
    speed: f32,
}

/// Enables live sample preview of a curve being edited.
#[derive(Component)]
struct PreviewSample(Entity);

/// System that animates the x position of entities with [AnimateX] using a [LookupCurve].
fn animate(
    mut animate: Query<(&mut Transform, &mut AnimateX, Option<&PreviewSample>)>,
    mut editors: Query<&mut LookupCurveEditor>,
    curves: Res<Assets<LookupCurve>>,
    time: Res<Time>,
) {
    for (mut transform, mut animate, preview) in animate.iter_mut() {
        // update t
        animate.t += animate.dir * animate.speed * time.delta_secs();

        // reverse direction at the ends
        if animate.t >= 1.0 {
            animate.dir = -1.0;
            animate.t = 1.0;
        }
        if animate.t <= 0.0 {
            animate.dir = 1.0;
            animate.t = 0.0;
        }

        // get the curve asset
        let Some(curve) = curves.get(&animate.curve) else {
            continue;
        };

        // update x position using the curve
        // using a cache may improve performance of coherent lookups if the curve has many knots
        // but as always, if you want to ensure optimal performance, benchmark and see what works best for your use case
        transform.translation.x = animate.from
            + (animate.to - animate.from) * curve.lookup_cached(animate.t, &mut animate.cache);

        // update the editor preview sample if `PreviewSample` is present
        if let Some(mut editor) = preview.and_then(|p| editors.get_mut(p.0).ok()) {
            editor.sample = Some(animate.t);
        }
    }
}
