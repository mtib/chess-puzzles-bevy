//! chess-puzzles-bevy — native Bevy twin of the Omarchy chess puzzle panel.

mod model;
mod paint;
mod paths;
mod pieces;
mod state;
mod theme;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::image::{Image, ImageSampler};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::render::view::{DebandDither, Tonemapping};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{futures::check_ready, IoTaskPool, Task};
use bevy::text::{FontSize, FontWeight, LetterSpacing};
use bevy::ui::RelativeCursorPosition;
use bevy::window::{CursorLeft, CursorMoved, WindowResized, WindowScaleFactorChanged};
use bevy::winit::{UpdateMode, WinitSettings};
use model::PuzzleRecord;
use paint::{paint_board, piece_size, BoardView};
use pieces::{PieceArt, PieceSprites};
use state::{PanelState, Tone};
use std::time::{Duration, Instant};
use theme::ui;

/// Logical px per square, before the display scale. The board shrinks
/// below MIN_CELL only when the window is forced smaller than its minimum
/// size (tiling), so it never overlaps the controls.
const MIN_CELL: f32 = 22.0;
const MAX_CELL: f32 = 96.0;
const FLOOR_CELL: f32 = 4.0;
/// Logical px kept free around the board inside its slot.
const BOARD_MARGIN: f32 = 8.0;
/// Logical height of everything around the board (title, status, buttons,
/// padding, board margins): about 200 px (218 measured at 1.6x, less the
/// removed meta line), plus slack.
const CONTROLS_H: f32 = 220.0;
/// Selection ring width, logical px.
const RING: f32 = 2.0;
/// Longest timer step per frame. The first frame after an idle wait reports
/// the whole wait as its delta; unclamped, a reply scheduled by that frame's
/// click would play instantly.
const MAX_TICK_MS: f32 = 50.0;
/// How long to keep rendering after the last activity. A couple of frames
/// is not enough: after a resize the rebuilt swapchain can drop its first
/// presents, and the window stays black until the next input.
const SETTLE_TAIL: Duration = Duration::from_millis(300);
/// Wake-up interval while idle, as a safety net; input wakes it at once.
const IDLE_WAKE: Duration = Duration::from_secs(60);

#[derive(Resource)]
struct PanelRes(PanelState);

/// The in-flight puzzle fetch, run on the IO pool so the window never blocks.
#[derive(Resource, Default)]
struct FetchJob(Option<Task<Option<PuzzleRecord>>>);

#[derive(Resource)]
struct PieceSource(PieceArt);

/// Where the board sits, in whole physical pixels: its top-left corner and
/// the texels per square. The texture is exactly as big as the board on
/// screen and starts on a pixel boundary, so every texel is one pixel.
#[derive(Resource, Clone, Copy, PartialEq, Default)]
struct BoardGeom {
    cell: u32,
    left: f32,
    top: f32,
    scale: f32,
}

impl BoardGeom {
    fn texels(&self) -> u32 {
        self.cell * 8
    }
}

#[derive(Resource)]
struct UiFont(Handle<Font>);

#[derive(Component)]
struct BoardSlot;
#[derive(Component)]
struct BoardSprite;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct SolvedText;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Action {
    NewPuzzle,
    ShowSolution,
}

impl Action {
    fn primary(self) -> bool {
        self == Action::NewPuzzle
    }
}

/// The button the mouse went down on; it fires if the mouse comes up on it.
#[derive(Resource, Default)]
struct ArmedButton(Option<Entity>);

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "chess puzzles".into(),
                        // Wayland app_id / X11 WM_CLASS, for window rules.
                        name: Some("chess-puzzles".into()),
                        // Preferred size in logical px (the scale factor is 1
                        // until the window exists, so these come out logical).
                        resolution: (340u32, 484u32).into(),
                        // Room for the controls plus a board of MIN_CELL squares.
                        resize_constraints: WindowResizeConstraints {
                            min_width: 8.0 * MIN_CELL + 2.0 * (16.0 + BOARD_MARGIN),
                            min_height: 8.0 * MIN_CELL + CONTROLS_H,
                            ..default()
                        },
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin { file_path: paths::assets_dir().to_string_lossy().into_owned(), ..default() }),
        )
        // Starts continuous; keep_awake drops to event-driven sleep once
        // nothing changes.
        .insert_resource(WinitSettings::continuous())
        .insert_resource(ClearColor(ui(theme::BG)))
        .insert_resource(PanelRes(PanelState::new()))
        .insert_resource(PieceSource(PieceArt::load(&paths::assets_dir().join("pieces"))))
        .init_resource::<FetchJob>()
        .init_resource::<BoardGeom>()
        .init_resource::<ArmedButton>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                layout_board,
                drive_fetch,
                board_input,
                keyboard_input,
                buttons,
                tick,
                update_ui,
                repaint_board,
                keep_awake,
            )
                .chain(),
        )
        .run();
}

fn text(font: &UiFont, value: impl Into<String>, size: f32, weight: FontWeight, color: theme::Rgba) -> impl Bundle {
    (
        Text::new(value),
        TextFont { font: font.0.clone().into(), font_size: FontSize::Px(size), weight, ..default() },
        TextColor(ui(color)),
    )
}

/// An UPPERCASE tracked label (labels, buttons, status words).
fn label(font: &UiFont, value: &str, color: theme::Rgba) -> impl Bundle {
    (text(font, value, 11.0, FontWeight::BOLD, color), LetterSpacing::Px(2.0))
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, asset_server: Res<AssetServer>) {
    // Flat colours stay exact: no tonemapping curve, no deband noise.
    commands.spawn((Camera2d, Tonemapping::None, DebandDither::Disabled));
    let font = UiFont(asset_server.load("fonts/JetBrainsMono-Variable.ttf"));
    // A world sprite rather than a UI image: UI layout rounds edges from
    // fractional positions, which can stretch the board by a pixel.
    commands.spawn((Sprite::from_image(images.add(board_image(8))), BoardSprite));

    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(16.0), Val::Px(14.0)),
            ..default()
        })
        .with_children(|col| {
            // Lowercase wordmark, solved count as a muted label beneath.
            col.spawn((text(&font, "chess puzzles", 24.0, FontWeight::BOLD, theme::HEADING), LetterSpacing::Px(-1.2)));
            col.spawn((label(&font, "", theme::MUTED), SolvedText, Node { margin: UiRect::top(Val::Px(4.0)), ..default() }));

            // Free space for the board; layout_board places the sprite in it.
            col.spawn((
                Node { flex_grow: 1.0, width: Val::Percent(100.0), min_height: Val::Px(0.0), ..default() },
                BoardSlot,
            ));

            col.spawn((text(&font, "", 14.0, FontWeight::NORMAL, theme::TEXT), LetterSpacing::Px(0.0), StatusText));

            col.spawn(Node { column_gap: Val::Px(8.0), margin: UiRect::top(Val::Px(12.0)), ..default() })
                .with_children(|row| {
                    spawn_button(row, &font, "SHOW SOLUTION", Action::ShowSolution);
                    spawn_button(row, &font, "NEW PUZZLE", Action::NewPuzzle);
                });
        });
    commands.insert_resource(font);
}

fn spawn_button(parent: &mut RelatedSpawnerCommands<ChildOf>, font: &UiFont, value: &str, action: Action) {
    parent
        .spawn((
            Node {
                height: Val::Px(34.0),
                padding: UiRect::horizontal(Val::Px(16.0)),
                border: UiRect::all(Val::Px(1.0)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor::default(),
            BorderColor::default(),
            RelativeCursorPosition::default(),
            action,
        ))
        .with_children(|btn| {
            btn.spawn(label(font, value, theme::TEXT));
        });
}

fn board_image(texels: u32) -> Image {
    let mut image = Image::new(
        Extent3d { width: texels, height: texels, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (texels * texels * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Texels map 1:1 to physical pixels; never filter.
    image.sampler = ImageSampler::nearest();
    image
}

// ------------------------------------------------------------------ systems

/// Sizes and places the board in the free slot: whole physical pixels per
/// square, top-left corner on a pixel boundary.
fn layout_board(
    windows: Query<&Window>,
    slot: Query<(&ComputedNode, &UiGlobalTransform), With<BoardSlot>>,
    mut sprite: Query<(&mut Transform, &mut Sprite), With<BoardSprite>>,
    mut geom: ResMut<BoardGeom>,
) {
    let (Ok(window), Ok((slot, slot_at)), Ok((mut transform, mut sprite))) =
        (windows.single(), slot.single(), sprite.single_mut())
    else {
        return;
    };
    let scale = window.scale_factor();
    let avail = (slot.size() - Vec2::splat(2.0 * BOARD_MARGIN * scale)).min_element();
    let cell = (avail / 8.0).floor().clamp(FLOOR_CELL * scale, MAX_CELL * scale) as u32;
    let board = (cell * 8) as f32;
    let corner = (slot_at.translation - board / 2.0).round();
    geom.set_if_neq(BoardGeom { cell, left: corner.x, top: corner.y, scale });

    // World space: origin at the window centre, y up, logical units.
    let window_px = Vec2::new(window.physical_width() as f32, window.physical_height() as f32);
    let centre = (corner + board / 2.0 - window_px / 2.0) / scale;
    let translation = Vec3::new(centre.x, -centre.y, 0.0);
    if transform.translation != translation {
        transform.translation = translation;
    }
    let size = Some(Vec2::splat(board / scale));
    if sprite.custom_size != size {
        sprite.custom_size = size;
    }
}

/// Starts requested fetches on the IO pool and hands results back.
fn drive_fetch(mut panel: ResMut<PanelRes>, mut job: ResMut<FetchJob>) {
    if let Some(task) = job.0.as_mut()
        && let Some(record) = check_ready(task)
    {
        job.0 = None;
        // A newer request (e.g. "new puzzle" pressed mid-fetch) supersedes this result.
        if !panel.0.fetch_pending() {
            panel.0.puzzle_loaded(record);
        }
    }
    if job.0.is_none()
        && let Some(elo) = panel.0.take_fetch_request()
    {
        job.0 = Some(IoTaskPool::get().spawn(async move { state::fetch_puzzle(elo) }));
    }
}

fn board_input(
    mut panel: ResMut<PanelRes>,
    windows: Query<&Window>,
    geom: Res<BoardGeom>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Ok(window) = windows.single() else { return };
    let cell = window.physical_cursor_position().and_then(|p| {
        let col = ((p.x - geom.left) / geom.cell as f32).floor();
        let row = ((p.y - geom.top) / geom.cell as f32).floor();
        ((0.0..8.0).contains(&col) && (0.0..8.0).contains(&row))
            .then(|| model::square_at(row as usize, col as usize, panel.0.is_flipped()))
    });
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);

    // Shift+drag draws an arrow; a plain press plays / clears.
    if mouse.just_pressed(MouseButton::Left)
        && let Some(idx) = cell
    {
        if shift {
            panel.0.begin_arrow(idx);
        } else {
            panel.0.click_square(idx);
        }
    }
    if panel.0.drag_arrow.is_some() {
        if !mouse.pressed(MouseButton::Left) {
            panel.0.end_arrow();
        } else if let Some(idx) = cell
            && panel.0.drag_arrow.is_some_and(|(_, to)| to != idx)
        {
            panel.0.drag_arrow_to(idx);
        }
    }
}

/// n new puzzle, s show solution, f flip board.
fn keyboard_input(mut panel: ResMut<PanelRes>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyN) {
        panel.0.request_puzzle();
    }
    if keys.just_pressed(KeyCode::KeyS) {
        panel.0.show_solution();
    }
    if keys.just_pressed(KeyCode::KeyF) {
        panel.0.toggle_flip();
    }
}

/// Click handling and the hover look: the primary button is an accent slab
/// that lightens; secondary buttons invert to white.
fn buttons(
    mut panel: ResMut<PanelRes>,
    mut armed: ResMut<ArmedButton>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut buttons: Query<(Entity, &Action, &RelativeCursorPosition, &mut BackgroundColor, &mut BorderColor, &Children)>,
    mut labels: Query<&mut TextColor>,
) {
    for (entity, action, cursor, mut bg, mut border, children) in &mut buttons {
        let enabled = match action {
            Action::NewPuzzle => true,
            Action::ShowSolution => panel.0.can_show_solution(),
        };
        let hovered = enabled && cursor.cursor_over();
        if hovered && mouse.just_pressed(MouseButton::Left) {
            armed.0 = Some(entity);
        }
        if mouse.just_released(MouseButton::Left) && armed.0 == Some(entity) {
            armed.0 = None;
            if hovered {
                match action {
                    Action::NewPuzzle => panel.0.request_puzzle(),
                    Action::ShowSolution => panel.0.show_solution(),
                }
            }
        }

        let (fill, edge, ink) = match (action.primary(), enabled, hovered) {
            (true, _, false) => (theme::ACCENT, theme::ACCENT, theme::BG),
            (true, _, true) => (theme::ACCENT_HOVER, theme::ACCENT_HOVER, theme::BG),
            (false, false, _) => (theme::CONTROL, theme::LINE_STRONG, theme::FAINT),
            (false, true, false) => (theme::CONTROL, theme::LINE_STRONG, theme::TEXT),
            (false, true, true) => (theme::INVERT, theme::INVERT, theme::BG),
        };
        bg.set_if_neq(BackgroundColor(ui(fill)));
        border.set_if_neq(BorderColor::all(ui(edge)));
        for child in children {
            if let Ok(mut color) = labels.get_mut(*child) {
                color.set_if_neq(TextColor(ui(ink)));
            }
        }
    }
}

fn tick(time: Res<Time>, mut panel: ResMut<PanelRes>) {
    panel.0.tick((time.delta_secs() * 1000.0).min(MAX_TICK_MS));
}

/// UI or board entities whose change still has to work through layout,
/// text shaping or rendering.
type Unsettled = Or<(Changed<ComputedNode>, Changed<Node>, Changed<Text>, Changed<Transform>, Changed<Sprite>)>;

/// Requests the next frame while anything is in motion: countdowns, fades, a
/// running fetch, the font loading, input, and any change still settling
/// (a resize takes several frames to go from layout to board repaint to
/// screen). Once a frame changes nothing, the event loop sleeps.
#[allow(clippy::too_many_arguments)]
fn keep_awake(
    panel: Res<PanelRes>,
    job: Res<FetchJob>,
    geom: Res<BoardGeom>,
    font: Res<UiFont>,
    assets: Res<AssetServer>,
    unsettled: Query<(), Unsettled>,
    mut images: MessageReader<AssetEvent<Image>>,
    mut cursor: MessageReader<CursorMoved>,
    mut left: MessageReader<CursorLeft>,
    mut clicks: MessageReader<MouseButtonInput>,
    mut keys: MessageReader<KeyboardInput>,
    mut resized: MessageReader<WindowResized>,
    mut rescaled: MessageReader<WindowScaleFactorChanged>,
    mut last_active: Local<Option<Instant>>,
    mut winit: ResMut<WinitSettings>,
) {
    let input = cursor.read().count()
        + left.read().count()
        + clicks.read().count()
        + keys.read().count()
        + resized.read().count()
        + rescaled.read().count()
        > 0;
    let settling = geom.is_changed() || !unsettled.is_empty() || images.read().count() > 0;
    let busy = panel.0.is_animating()
        || panel.0.fetch_pending()
        || job.0.is_some()
        || !assets.is_loaded_with_dependencies(font.0.id());
    if input || settling || busy {
        *last_active = Some(Instant::now());
    }
    // Awake means Continuous rather than RequestRedraw: on Wayland a redraw
    // request is paced by frame callbacks and can be lost before the window
    // is configured, leaving it black until the next input event. Switching
    // the mode makes winit run the next update itself.
    let mode = if last_active.is_some_and(|t| t.elapsed() < SETTLE_TAIL) {
        UpdateMode::Continuous
    } else {
        UpdateMode::reactive_low_power(IDLE_WAKE)
    };
    if winit.focused_mode != mode {
        debug!(
            "{} (input {input}, settling {settling}, busy {busy}, board {}px at {},{})",
            if mode == UpdateMode::Continuous { "awake" } else { "asleep" },
            geom.cell * 8,
            geom.left,
            geom.top,
        );
        winit.focused_mode = mode;
        winit.unfocused_mode = mode;
    }
}

/// Writes only on a real change: a plain `&mut` access through `Mut` marks
/// the text changed and re-shapes it, even when the value is equal.
fn set_text(mut text: Mut<Text>, value: String) {
    if text.0 != value {
        text.0 = value;
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_ui(
    panel: Res<PanelRes>,
    mut status: Query<(&mut Text, &mut TextColor, &mut TextFont), With<StatusText>>,
    mut solved_text: Query<&mut Text, (With<SolvedText>, Without<StatusText>)>,
) {
    let s = &panel.0;
    if let Ok((text, mut color, mut font)) = status.single_mut() {
        set_text(text, s.status.clone());
        let (ink, weight) = match s.tone {
            Tone::Normal => (theme::TEXT, FontWeight::NORMAL),
            Tone::Muted => (theme::MUTED, FontWeight::NORMAL),
            Tone::Good => (theme::TEXT, FontWeight::BOLD),
            Tone::Bad => (theme::ACCENT, FontWeight::BOLD),
        };
        color.set_if_neq(TextColor(ui(ink)));
        if font.weight != weight {
            font.weight = weight;
        }
    }
    if let Ok(text) = solved_text.single_mut() {
        set_text(text, format!("{} SOLVED", s.solved));
    }
}

/// Repaints the board texture, only when what it shows (or its size) changed.
#[allow(clippy::too_many_arguments)]
fn repaint_board(
    panel: Res<PanelRes>,
    art: Res<PieceSource>,
    geom: Res<BoardGeom>,
    mut images: ResMut<Assets<Image>>,
    mut board: Query<&mut Sprite, With<BoardSprite>>,
    mut sprites: Local<Option<PieceSprites>>,
    mut painted: Local<Option<(BoardView, u32, f32)>>,
) {
    if geom.cell == 0 {
        return;
    }
    let Ok(mut sprite) = board.single_mut() else { return };
    let view = BoardView::of(&panel.0);
    // Placement alone doesn't change the picture: key on cell size and scale.
    let key = (view, geom.cell, geom.scale);
    if painted.as_ref() == Some(&key) {
        return;
    }

    let size = piece_size(geom.cell);
    if sprites.as_ref().is_none_or(|s| s.size != size) {
        *sprites = Some(art.0.rasterize(size));
    }
    let ring = (RING * geom.scale).round().max(1.0) as u32;
    let data = paint_board(&key.0, sprites.as_ref().unwrap(), geom.cell, ring);

    let texels = geom.texels();
    if let Some(mut image) = images.get_mut(&sprite.image)
        && image.width() == texels
    {
        image.data = Some(data);
    } else {
        // Size changed (window resized or moved between displays).
        let mut image = board_image(texels);
        image.data = Some(data);
        sprite.image = images.add(image);
    }
    *painted = Some(key);
}
