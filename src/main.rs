//! chess-puzzles-bevy — native Bevy twin of the Omarchy chess puzzle panel.

mod model;
mod paint;
mod state;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::{FontSize, FontSource};
use model::Color as Side;
use paint::paint_board;
use state::{PanelState, Phase, ELO_MAX, ELO_MIN, ELO_STEP};

use bevy::image::Image;
use std::collections::HashMap;

// All layout math is in LOGICAL pixels and derived from the ACTUAL window
// size each frame, so the panel adjusts to any window size.
// Chrome above the board: header (38) + ELO row (26) + slider track (26).
const CHROME_H: f32 = 90.0;
// Status line + buttons below the board.
const BOTTOM_H: f32 = 72.0;
const SLIDER_W: f32 = 330.0;
const SLIDER_Y: f32 = 68.0 + 11.0; // track occupies y 68..90; center = 79
const MIN_CELL: u32 = 22;
const MAX_CELL: u32 = 96;

/// Board geometry derived from the live window size and display scale.
/// On HiDPI (retina) the texture is painted at `paint_cell` texels per square
/// — cell * scale — while the sprite displays at logical `cell` px, so the
/// pieces keep full physical-pixel detail instead of being upscaled.
#[derive(Resource, Clone, Copy)]
struct BoardLayout {
    cell: u32,       // logical px per square
    paint_cell: u32, // texels per square (cell * display scale factor)
    x0: f32,         // board left edge (logical)
    top: f32,        // board top edge (logical)
    size: f32,       // board display size (logical, cell * 8)
}

impl BoardLayout {
    fn compute(w: f32, h: f32, scale: f32) -> Self {
        let avail_w = (w - 16.0).max(64.0);
        let avail_h = (h - CHROME_H - BOTTOM_H - 16.0).max(64.0);
        let cell = ((avail_w.min(avail_h) / 8.0).floor() as u32).clamp(MIN_CELL, MAX_CELL);
        let size = cell as f32 * 8.0;
        let x0 = ((w - size) / 2.0).max(0.0);
        let top = CHROME_H + (h - BOTTOM_H - CHROME_H - size) / 2.0;
        let paint_cell = ((cell as f32 * scale).round() as u32).max(cell);
        BoardLayout { cell, paint_cell, x0, top, size }
    }
}

#[derive(Resource)]
struct PanelRes(PanelState);

#[derive(Resource)]
struct BoardImageHandle(Handle<Image>);

#[derive(Resource, Default)]
struct CurrentCell(u32);

#[derive(Component)]
struct BoardSprite;

#[derive(Resource)]
struct PieceImages(HashMap<String, image::RgbaImage>);

#[derive(Resource)]
struct Cursor {
    x: f32,
    y: f32, // logical px, top-down
    inside: bool,
}

impl Cursor {
    fn new() -> Self {
        Cursor { x: 0.0, y: 0.0, inside: false }
    }
}

#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct EloValueText;
#[derive(Component)]
struct SolvedText;
#[derive(Component)]
struct SliderFill;
#[derive(Component)]
struct SliderKnob;
#[derive(Component)]
struct BtnNew;
#[derive(Component)]
struct BtnSolution;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Chess Puzzles".into(),
                resolution: (400u32, 560u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0))) // pure black
        .insert_resource(PanelRes(PanelState::new()))
        .insert_resource(Cursor::new())
        .add_systems(Startup, (setup, load_pieces, initial_fetch))
        .add_systems(Update, position_board)
        .add_systems(
            Update,
            (
                track_cursor,
                board_input,
                ui_interactions,
                timers,
                update_ui,
                repaint_board,
            )
                .chain(),
        )
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, asset_server: Res<AssetServer>, windows: Query<&Window>) {
    commands.spawn(Camera2d);

    // Board texture, created empty and repainted from PanelState at the
    // window-derived cell size.
    let (win_w, win_h, scale) = windows
        .single()
        .map(|w| (w.width(), w.height(), w.scale_factor()))
        .unwrap_or((470.0, 640.0, 1.0));
    let layout = BoardLayout::compute(win_w, win_h, scale);
    let tex = layout.paint_cell * 8;
    let initial = Image::new(
        Extent3d { width: tex, height: tex, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (tex * tex * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(initial);
    commands.insert_resource(BoardImageHandle(handle.clone()));
    commands.insert_resource(CurrentCell(layout.paint_cell));
    commands.insert_resource(layout);

    commands.spawn((
        Sprite {
            image: handle,
            custom_size: Some(Vec2::splat(layout.size)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
        BoardSprite,
    ));

    // ---- UI chrome
    let font: Handle<Font> = asset_server.load("fonts/JetBrainsMonoNerdFont-Regular.ttf");

    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|col| {
            // Header: knight logo + title + solved count.
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                margin: UiRect::all(Val::Px(6.0)),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new("CHESS PUZZLES"),
                    TextFont {
                        font: FontSource::Handle(font.clone()),
                        font_size: FontSize::Px(19.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.92)),
                ));
                row.spawn((
                    Text::new(""),
                    TextFont {
                        font: FontSource::Handle(font.clone()),
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.063, 0.72, 0.505)), // #10b981 electric green
                    SolvedText,
                ));
            });

            // ELO label + value.
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                margin: UiRect::top(Val::Px(10.0)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new("PUZZLE ELO"),
                    TextFont {
                        font: FontSource::Handle(font.clone()),
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.54, 0.56, 0.60)), // dim grey
                ));
                row.spawn((
                    Text::new("1500"),
                    TextFont {
                        font: FontSource::Handle(font.clone()),
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.92)),
                    EloValueText,
                ));
            });

            // Slider track: base track + fill + knob, all absolutely
            // positioned so they overlay the same 6px band.
            col.spawn(Node {
                width: Val::Px(SLIDER_W),
                height: Val::Px(22.0),
                margin: UiRect::top(Val::Px(4.0)),
                ..default()
            })
            .with_children(|track| {
                track.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(8.0),
                        left: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.165, 0.165, 0.188)), // #2a2a30
                ));
                track.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(8.0),
                        left: Val::Px(0.0),
                        width: Val::Percent(0.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(1.0, 0.322, 0.196)), // coral #ff5232
                    SliderFill,
                ));
                track.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(4.0),
                        left: Val::Px(0.0),
                        width: Val::Px(14.0),
                        height: Val::Px(14.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.90, 0.91, 0.94)),
                    SliderKnob,
                ));
            });

            // Flexible spacer: the world-space board lives in this gap at
            // any window size, with status + buttons anchored to the bottom.
            col.spawn(Node { flex_grow: 1.0, ..default() });

            // Status line.
            col.spawn((
                Node {
                    margin: UiRect::new(Val::Px(0.0), Val::Px(0.0), Val::Px(6.0), Val::Px(6.0)),
                    ..default()
                },
                Text::new(""),
                TextFont {
                    font: FontSource::Handle(font.clone()),
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.92)),
                StatusText,
            ));

            // Buttons.
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                margin: UiRect::bottom(Val::Px(10.0)),
                ..default()
            })
            .with_children(|row| {
                spawn_button(row, font.clone(), "SHOW SOLUTION", BtnSolution);
                spawn_button(row, font.clone(), "NEW PUZZLE", BtnNew);
            });
        });
}

fn spawn_button(parent: &mut RelatedSpawnerCommands<ChildOf>, font: Handle<Font>, label: &str, marker: impl Component) {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.08, 0.08, 0.10)), // flat dark fill
            BorderColor::all(Color::srgb(0.23, 0.23, 0.26)), // 1px grey outline
            Interaction::default(),
            marker,
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new(label.to_string()),
                TextFont {
                    font: FontSource::Handle(font),
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.92)),
            ));
        });
}

fn load_pieces(mut commands: Commands) {
    // Piece PNGs are loaded straight off disk — the painter works on raw
    // pixels, not on Bevy assets.
    let mut map = HashMap::new();
    for kind in ["p", "n", "b", "r", "q", "k"] {
        for color in ["w", "b"] {
            let name = format!("{kind}{color}");
            if let Ok(img) = image::open(format!("assets/pieces/{name}.png")) {
                map.insert(name, img.to_rgba8());
            }
        }
    }
    commands.insert_resource(PieceImages(map));
}

fn initial_fetch(mut panel: ResMut<PanelRes>) {
    panel.0.fetch_puzzle();
}

// ------------------------------------------------------------------ systems

fn track_cursor(mut commands: Commands, mut cursor: ResMut<Cursor>, windows: Query<&Window>) {
    let Ok(window) = windows.single() else { return };
    if let Some(pos) = window.cursor_position() {
        // cursor_position is logical and top-down relative to the window.
        cursor.x = pos.x;
        cursor.y = pos.y;
        cursor.inside = true;
    } else {
        cursor.inside = false;
    }
    let layout = BoardLayout::compute(window.width(), window.height(), window.scale_factor());
    commands.insert_resource(layout);
}

fn board_cell_at(state: &PanelState, cursor: &Cursor, layout: &BoardLayout) -> Option<usize> {
    if !cursor.inside {
        return None;
    }
    let cell = layout.cell as f32;
    let x = cursor.x - layout.x0;
    let y = cursor.y - layout.top;
    if x < 0.0 || y < 0.0 || x >= layout.size || y >= layout.size {
        return None;
    }
    let col = (x / cell) as usize;
    let row = (y / cell) as usize;
    let flip = state.user_color == Side::Black || state.flip_view;
    Some(if flip { (7 - row) * 8 + (7 - col) } else { row * 8 + col })
}

fn board_input(
    mut panel: ResMut<PanelRes>,
    cursor: Res<Cursor>,
    layout: Res<BoardLayout>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let cell = board_cell_at(&panel.0, &cursor, &layout);

    // Shift+drag arrow drawing; plain press plays / clears.
    if mouse.just_pressed(MouseButton::Left) {
        if let Some(idx) = cell {
            if shift {
                panel.0.drag_arrow = Some((idx, idx));
                return;
            }
            panel.0.click_square(idx);
        }
    }
    if let Some((from, _)) = panel.0.drag_arrow {
        if mouse.pressed(MouseButton::Left) {
            if let Some(idx) = cell {
                panel.0.drag_arrow = Some((from, idx));
            }
        } else {
            if let Some((from, to)) = panel.0.drag_arrow {
                if from != to {
                    panel.0.user_arrows.push((from, to));
                }
            }
            panel.0.drag_arrow = None;
        }
    }

    // Keyboard: n new puzzle, s show solution, f flip board.
    if keys.just_pressed(KeyCode::KeyN) {
        panel.0.fetch_puzzle();
    }
    if keys.just_pressed(KeyCode::KeyS) {
        panel.0.show_solution();
    }
    if keys.just_pressed(KeyCode::KeyF) {
        panel.0.flip_view = !panel.0.flip_view;
    }
}

fn ui_interactions(
    mut panel: ResMut<PanelRes>,
    cursor: Res<Cursor>,
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut interactions: Query<
        (&Interaction, Option<&BtnNew>, Option<&BtnSolution>),
        Changed<Interaction>,
    >,
) {
    for (interaction, new_btn, solution_btn) in interactions.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if new_btn.is_some() && panel.0.phase != Phase::Loading {
            panel.0.fetch_puzzle();
        }
        if solution_btn.is_some() {
            panel.0.show_solution();
        }
    }

    // ELO slider: press or hold anywhere on the track to set the value.
    let win_w = windows.single().map(|w| w.width()).unwrap_or(470.0);
    let track_x0 = (win_w - SLIDER_W) / 2.0;
    let track_x1 = track_x0 + SLIDER_W;
    let near_track = cursor.inside
        && cursor.x >= track_x0 - 8.0
        && cursor.x <= track_x1 + 8.0
        && (cursor.y - SLIDER_Y).abs() <= 14.0;
    if mouse.pressed(MouseButton::Left) && near_track {
        let frac = ((cursor.x - track_x0) / SLIDER_W).clamp(0.0, 1.0);
        let raw = ELO_MIN as f32 + frac * (ELO_MAX - ELO_MIN) as f32;
        let elo = ((raw / ELO_STEP as f32).round() as i64 * ELO_STEP).clamp(ELO_MIN, ELO_MAX);
        panel.0.set_elo(elo);
    }
}

fn timers(time: Res<Time>, mut panel: ResMut<PanelRes>) {
    let dt = time.delta_secs() * 1000.0;
    let s = &mut panel.0;

    if s.reply_timer > 0.0 {
        s.reply_timer -= dt;
        if s.reply_timer <= 0.0 {
            s.reply_timer = 0.0;
            s.play_reply();
        }
    }
    if s.solution_timer > 0.0 {
        s.solution_timer -= dt;
        if s.solution_timer <= 0.0 {
            s.solution_timer = 0.0;
            s.play_solution_step();
        }
    }
    if s.flash_timer > 0.0 {
        s.flash_timer -= dt;
        if s.flash_timer <= 0.0 {
            s.flash_timer = 0.0;
            s.wrong_flash = None;
        }
    }
    if let Some(arrow) = s.auto_arrow.as_mut() {
        arrow.age_ms += dt;
        // Show 900ms, fade 500ms.
        arrow.alpha = if arrow.age_ms < 900.0 {
            1.0
        } else {
            (1.0 - (arrow.age_ms - 900.0) / 500.0).max(0.0)
        };
        if arrow.alpha <= 0.0 {
            s.auto_arrow = None;
        }
    }
}

fn update_ui(
    mut texts: Query<(
        &mut Text,
        Option<&StatusText>,
        Option<&EloValueText>,
        Option<&SolvedText>,
    )>,
    mut slider: Query<(&mut Node, Option<&SliderFill>, Option<&SliderKnob>)>,
    mut status_colors: Query<&mut TextColor, With<StatusText>>,
    panel: Res<PanelRes>,
) {
    let status = panel.0.status.clone();
    let elo = panel.0.target_elo;
    let solved = panel.0.solved;

    for (mut text, status_m, elo_m, solved_m) in texts.iter_mut() {
        if status_m.is_some() {
            text.0 = status.clone();
        }
        if elo_m.is_some() {
            text.0 = elo.to_string();
        }
        if solved_m.is_some() {
            text.0 = if solved > 0 { format!("{solved} solved") } else { String::new() };
        }
    }

    let frac = (elo - ELO_MIN) as f32 / (ELO_MAX - ELO_MIN) as f32;
    for (mut node, fill, knob) in slider.iter_mut() {
        if fill.is_some() {
            node.width = Val::Percent(frac * 100.0);
        }
        if knob.is_some() {
            node.left = Val::Px(frac * SLIDER_W - 7.0);
        }
    }

    for mut color in status_colors.iter_mut() {
        color.0 = if panel.0.phase == Phase::Won {
            Color::srgb(0.063, 0.72, 0.505) // #10b981
        } else {
            Color::srgb(0.90, 0.91, 0.94)
        };
    }
}

/// Keep the world-space board where the layout says, at the right size —
/// every frame, so any window size just works.
fn position_board(
    layout: Res<BoardLayout>,
    mut sprite: Query<(&mut Transform, &mut Sprite), With<BoardSprite>>,
    windows: Query<&Window>,
) {
    let Ok(window) = windows.single() else { return };
    let win_h = window.height();
    let Ok((mut transform, mut sprite)) = sprite.single_mut() else { return };
    // World y is up-positive; screen top-down board center:
    let center_y_screen = layout.top + layout.size / 2.0;
    transform.translation.y = win_h / 2.0 - center_y_screen;
    transform.translation.x = layout.x0 + layout.size / 2.0 - window.width() / 2.0;
    sprite.custom_size = Some(Vec2::splat(layout.size));
}

fn repaint_board(
    panel: Res<PanelRes>,
    pieces: Res<PieceImages>,
    layout: Res<BoardLayout>,
    mut current: ResMut<CurrentCell>,
    mut handle: ResMut<BoardImageHandle>,
    mut images: ResMut<Assets<Image>>,
    mut sprite: Query<&mut Sprite, With<BoardSprite>>,
    mut last: Local<Vec<u8>>,
) {
    let cell = layout.paint_cell;
    // Paint size changed (window resized or moved between displays): rebuild
    // the texture at the new resolution.
    if cell != current.0 {
        let tex = cell * 8;
        let image = Image::new(
            Extent3d { width: tex, height: tex, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![0; (tex * tex * 4) as usize],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        let new_handle = images.add(image);
        if let Ok(mut sp) = sprite.single_mut() {
            sp.image = new_handle.clone();
        }
        handle.0 = new_handle;
        last.clear();
        current.0 = cell;
    }
    let data = paint_board(&panel.0, &pieces.0, cell);
    if data == *last {
        return;
    }
    *last = data.clone();
    if let Some(mut image) = images.get_mut(&handle.0) {
        match &mut image.data {
            Some(buf) => *buf = data,
            None => image.data = Some(data),
        }
    }
}
