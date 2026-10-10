//! chess-puzzles-bevy — native Bevy twin of the Omarchy chess puzzle panel.

mod model;
mod paint;
mod state;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::text::{FontSize, FontSource};
use bevy::window::WindowResolution;
use model::Color as Side;
use paint::{paint_board, TEX};
use state::{PanelState, Phase, ELO_MAX, ELO_MIN, ELO_STEP};

use bevy::image::Image;
use std::collections::HashMap;

const WINDOW_W: f32 = 470.0;
const WINDOW_H: f32 = 740.0;
const BOARD_DISPLAY: f32 = 432.0; // 8 x 54px cells, integer like the QML one
const BOARD_TOP: f32 = 196.0;
const SLIDER_W: f32 = 330.0;
const SLIDER_Y: f32 = 122.0; // screen px (top-down) of the track center

#[derive(Resource)]
struct PanelRes(PanelState);

#[derive(Resource)]
struct BoardImageHandle(Handle<Image>);

#[derive(Resource)]
struct PieceImages(HashMap<String, image::RgbaImage>);

#[derive(Resource)]
struct Cursor {
    x: f32,
    y: f32, // screen px, top-down
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
                resolution: WindowResolution::new(WINDOW_W as u32, WINDOW_H as u32),
                resizable: false,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(PanelRes(PanelState::new()))
        .insert_resource(Cursor::new())
        .add_systems(Startup, (setup, load_pieces, initial_fetch))
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

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);

    // Board texture, created empty and repainted from PanelState.
    let initial = Image::new(
        Extent3d { width: TEX, height: TEX, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (TEX * TEX * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(initial);
    commands.insert_resource(BoardImageHandle(handle.clone()));

    let board_center_y = WINDOW_H / 2.0 - (BOARD_TOP + BOARD_DISPLAY / 2.0);
    commands.spawn((
        Sprite {
            image: handle,
            custom_size: Some(Vec2::splat(BOARD_DISPLAY)),
            ..default()
        },
        Transform::from_xyz(0.0, board_center_y, 0.0),
    ));

    // ---- UI chrome
    let font: Handle<Font> = asset_server.load("fonts/JetBrainsMonoNerdFont-Regular.ttf");
    let knight: Handle<Image> = asset_server.load("knight.png");

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
                    Node { width: Val::Px(26.0), height: Val::Px(26.0), ..default() },
                    ImageNode::new(knight),
                ));
                row.spawn((
                    Text::new("Chess Puzzles"),
                    TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(19.0), ..default() },
                    TextColor(Color::srgb(0.92, 0.92, 0.92)),
                ));
                row.spawn((
                    Text::new(""),
                    TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(13.0), ..default() },
                    TextColor(Color::srgb(0.55, 0.55, 0.55)),
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
                    Text::new("Puzzle ELO"),
                    TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(13.0), ..default() },
                    TextColor(Color::srgb(0.55, 0.55, 0.55)),
                ));
                row.spawn((
                    Text::new("1500"),
                    TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(13.0), ..default() },
                    TextColor(Color::srgb(0.92, 0.92, 0.92)),
                    EloValueText,
                ));
            });

            // Slider track: base track + fill + knob.
            col.spawn(Node {
                width: Val::Px(SLIDER_W),
                height: Val::Px(22.0),
                ..default()
            })
            .with_children(|track| {
                track.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.25, 0.27)),
                ));
                track.spawn((
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.91, 0.54, 0.05)),
                    SliderFill,
                ));
                track.spawn((
                    Node {
                        width: Val::Px(14.0),
                        height: Val::Px(14.0),
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        border_radius: BorderRadius::all(CornerRadius::all(Val::Px(7.0))),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.92, 0.92, 0.92)),
                    SliderKnob,
                ));
            });

            // Spacer that leaves room for the world-space board.
            col.spawn(Node { height: Val::Px(BOARD_TOP - 156.0 + BOARD_DISPLAY), ..default() });

            // Status line.
            col.spawn((
                Node {
                    margin: UiRect::new(Val::Px(0.0), Val::Px(0.0), Val::Px(6.0), Val::Px(6.0)),
                    ..default()
                },
                Text::new(""),
                TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(14.0), ..default() },
                TextColor(Color::srgb(0.92, 0.92, 0.92)),
                StatusText,
            ));

            // Buttons.
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|row| {
                spawn_button(row, font.clone(), "Show solution", BtnSolution);
                spawn_button(row, font.clone(), "New puzzle", BtnNew);
            });
        });
}

fn spawn_button(parent: &mut RelatedSpawnerCommands<ChildOf>, font: Handle<Font>, label: &str, marker: impl Component) {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(CornerRadius::all(Val::Px(6.0))),
                ..default()
            },
            BackgroundColor(Color::srgb(0.16, 0.16, 0.17)),
            Interaction::default(),
            marker,
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new(label.to_string()),
                TextFont { font: FontSource::Handle(font), font_size: FontSize::Px(13.0), ..default() },
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

fn track_cursor(mut cursor: ResMut<Cursor>, windows: Query<&Window>) {
    let Ok(window) = windows.single() else { return };
    if let Some(pos) = window.cursor_position() {
        // cursor_position is top-down relative to the window.
        cursor.x = pos.x;
        cursor.y = pos.y;
        cursor.inside = true;
    } else {
        cursor.inside = false;
    }
}

fn board_cell_at(state: &PanelState, cursor: &Cursor) -> Option<usize> {
    if !cursor.inside {
        return None;
    }
    let cell = BOARD_DISPLAY / 8.0;
    let x = cursor.x - (WINDOW_W - BOARD_DISPLAY) / 2.0;
    let y = cursor.y - BOARD_TOP;
    if x < 0.0 || y < 0.0 || x >= BOARD_DISPLAY || y >= BOARD_DISPLAY {
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
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let cell = board_cell_at(&panel.0, &cursor);

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
    let track_x0 = (WINDOW_W - SLIDER_W) / 2.0;
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
            Color::srgb(0.91, 0.54, 0.05)
        } else {
            Color::srgb(0.92, 0.92, 0.92)
        };
    }
}

fn repaint_board(
    panel: Res<PanelRes>,
    pieces: Res<PieceImages>,
    handle: Res<BoardImageHandle>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Vec<u8>>,
) {
    let data = paint_board(&panel.0, &pieces.0);
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
