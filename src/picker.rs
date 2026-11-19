use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    mouse::MouseButton,
    pixels::Color,
    rect::Rect,
    render::{Canvas, RenderTarget},
};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 160;
const VISIBLE: usize = 3;
const ROW_TOP: i32 = 24;
const ROW_HEIGHT: i32 = 38;

// The native engine links these immutable arrays from src/fonts.c. Each of
// the 512 glyphs in latin_normal.latfont consists of four 8x8 two-bit tiles.
unsafe extern "C" {
    static gFontNormalLatinGlyphs: [u16; 512 * 32];
    static gFontNormalLatinGlyphWidths: [u8; 512];
}

fn glyph(c: char) -> usize {
    match c {
        'A'..='Z' => 0xbb + (c as usize - 'A' as usize),
        'a'..='z' => 0xd5 + (c as usize - 'a' as usize),
        '0'..='9' => 0xa1 + (c as usize - '0' as usize),
        ' ' => 0,
        'é' => 0x1b,
        '↑' => 0x79,
        '↓' => 0x7a,
        '!' => 0xab,
        '?' => 0xac,
        '.' => 0xad,
        '-' | '–' | '—' | '_' => 0xae,
        '·' => 0xaf,
        '…' => 0xb0,
        '“' => 0xb1,
        '”' => 0xb2,
        '‘' => 0xb3,
        '’' | '\'' => 0xb4,
        '♂' => 0xb5,
        '♀' => 0xb6,
        ',' => 0xb8,
        '/' => 0xba,
        ':' => 0xf0,
        'Ä' => 0xf1,
        'Ö' => 0xf2,
        'Ü' => 0xf3,
        'ä' => 0xf4,
        'ö' => 0xf5,
        'ü' => 0xf6,
        _ => 0xac,
    }
}

fn glyph_width(id: usize) -> i32 {
    // SAFETY: glyph always produces an index within the C font arrays.
    i32::from(unsafe { gFontNormalLatinGlyphWidths[id] })
}

fn text<T: RenderTarget>(
    canvas: &mut Canvas<T>,
    mut x: i32,
    y: i32,
    right: i32,
    value: &str,
    colors: (Color, Color),
) -> Result<(), String> {
    let fits = value.chars().map(|c| glyph_width(glyph(c))).sum::<i32>() <= right - x;
    let mut glyphs = value.chars().map(glyph).peekable();
    while let Some(mut id) = glyphs.next() {
        let width = glyph_width(id);
        // Reserve space for an ellipsis when a label extends past its row.
        let clipped = !fits && glyphs.peek().is_some() && x + width + glyph_width(0xb0) > right;
        if clipped {
            id = 0xb0;
        }
        let width = glyph_width(id);
        if x + width > right {
            break;
        }
        for py in 0..15 {
            for px in 0..width {
                let tile = (py / 8) * 2 + px / 8;
                let index = id * 32 + (tile * 8 + py % 8) as usize;
                // SAFETY: the tile index and row stay inside this glyph.
                let row = unsafe { gFontNormalLatinGlyphs[index] };
                match (row >> (14 - 2 * (px % 8))) & 3 {
                    1 => canvas.set_draw_color(colors.0),
                    2 => canvas.set_draw_color(colors.1),
                    _ => continue,
                }
                canvas.fill_rect(Rect::new(x + px, y + py, 1, 1))?;
            }
        }
        x += width;
        if clipped {
            break;
        }
    }
    Ok(())
}

fn paint<T: RenderTarget>(
    canvas: &mut Canvas<T>,
    saves: &[(String, String)],
    new_name: &str,
    selected: usize,
    first: usize,
) -> Result<(), String> {
    let ink = Color::RGB(49, 57, 82);
    let shadow = Color::RGB(205, 214, 221);
    let blue = Color::RGB(49, 82, 123);
    let highlight = Color::RGB(90, 164, 230);
    canvas.set_draw_color(Color::RGB(189, 205, 222));
    canvas.clear();
    canvas.set_draw_color(Color::RGB(198, 214, 230));
    for y in (20..140).step_by(8) {
        canvas.fill_rect(Rect::new(0, y, WIDTH, 4))?;
    }
    canvas.set_draw_color(blue);
    canvas.fill_rect(Rect::new(0, 0, WIDTH, 20))?;
    canvas.fill_rect(Rect::new(0, 140, WIDTH, 20))?;
    let light_text = (Color::WHITE, ink);
    text(canvas, 10, 2, 182, "Choose a save file", light_text)?;
    let position = format!("{}/{}", selected + 1, saves.len() + 1);
    let count_width: i32 = position.chars().map(|c| glyph_width(glyph(c))).sum();
    text(canvas, 230 - count_width, 2, 230, &position, light_text)?;
    for (row, index) in (first..=saves.len()).take(VISIBLE).enumerate() {
        let y = ROW_TOP + row as i32 * ROW_HEIGHT;
        let active = index == selected;
        canvas.set_draw_color(ink);
        canvas.fill_rect(Rect::new(7, y + 2, 226, 32))?;
        canvas.fill_rect(Rect::new(9, y, 222, 36))?;
        canvas.set_draw_color(if active {
            highlight
        } else {
            Color::RGB(148, 156, 172)
        });
        canvas.fill_rect(Rect::new(9, y + 2, 222, 32))?;
        canvas.set_draw_color(if active {
            Color::WHITE
        } else {
            Color::RGB(238, 242, 245)
        });
        canvas.fill_rect(Rect::new(11, y + 4, 218, 28))?;
        if active {
            canvas.set_draw_color(Color::RGB(213, 66, 66));
            for dx in 0..5 {
                canvas.fill_rect(Rect::new(15 + dx, y + 12 + dx, 1, (9 - 2 * dx) as u32))?;
            }
        }
        let (name, preview) = saves
            .get(index)
            .map(|(name, summary)| (name.as_str(), summary.as_str()))
            .unwrap_or(("New game", new_name));
        text(canvas, 25, y + 2, 225, name, (ink, shadow))?;
        text(canvas, 25, y + 16, 225, preview, (blue, shadow))?;
    }
    text(
        canvas,
        10,
        141,
        237,
        "↑↓ Choose    Z/Enter Open    X Back",
        light_text,
    )?;
    canvas.present();
    Ok(())
}

fn next_name(saves: &[(String, String)]) -> String {
    (1..)
        .map(|n| format!("Adventure {n}"))
        .find(|name| {
            !saves
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        })
        .unwrap()
}

/// Select an existing adventure or make a fresh slot; cancelling closes the launcher.
pub fn choose(saves: &[(String, String)]) -> Result<Option<String>, String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let window = video
        .window("Rustic Emerald", WIDTH * 3, HEIGHT * 3)
        .position_centered()
        .allow_highdpi()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    canvas
        .set_logical_size(WIDTH, HEIGHT)
        .map_err(|e| e.to_string())?;
    let new_name = next_name(saves);
    let mut events = sdl.event_pump()?;
    let mut selected = 0usize;
    let mut first = 0usize;
    loop {
        first = first.clamp(selected.saturating_sub(VISIBLE - 1), selected);
        let name = saves
            .get(selected)
            .map(|(name, _)| name)
            .unwrap_or(&new_name);
        canvas
            .window_mut()
            .set_title(&format!("Rustic Emerald — {name}"))
            .map_err(|e| e.to_string())?;
        paint(&mut canvas, saves, &new_name, selected, first)?;
        match events.wait_event() {
            Event::Quit { .. }
            | Event::Window {
                win_event: WindowEvent::Close,
                ..
            }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape | Keycode::X),
                ..
            } => return Ok(None),
            Event::KeyDown {
                keycode: Some(Keycode::Return | Keycode::KpEnter | Keycode::Z),
                repeat: false,
                ..
            } => return Ok(Some(name.clone())),
            Event::KeyDown {
                keycode: Some(Keycode::Up),
                ..
            } => selected = selected.saturating_sub(1),
            Event::KeyDown {
                keycode: Some(Keycode::Down),
                ..
            } => selected = (selected + 1).min(saves.len()),
            Event::KeyDown {
                keycode: Some(Keycode::Home),
                ..
            } => selected = 0,
            Event::KeyDown {
                keycode: Some(Keycode::End),
                ..
            } => selected = saves.len(),
            Event::MouseWheel { y, .. } if y > 0 => selected = selected.saturating_sub(1),
            Event::MouseWheel { y, .. } if y < 0 => selected = (selected + 1).min(saves.len()),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                clicks,
                ..
            } if (7..233).contains(&x)
                && (ROW_TOP..ROW_TOP + ROW_HEIGHT * VISIBLE as i32).contains(&y) =>
            {
                let row = (y - ROW_TOP) / ROW_HEIGHT;
                let index = first + row as usize;
                if (y - ROW_TOP) % ROW_HEIGHT < 36 && index <= saves.len() {
                    selected = index;
                    if clicks >= 2 {
                        return Ok(Some(
                            saves.get(index).map(|s| &s.0).unwrap_or(&new_name).clone(),
                        ));
                    }
                }
            }
            _ => (),
        }
    }
}
