mod maskers;
pub(crate) mod masks;

use crate::error::ViuResult;
use crate::printer::{adjust_offset, Printer};
use crate::Config;

use ansi_colours::ansi256_from_rgb;
use image::{DynamicImage, GenericImage, GenericImageView, ImageBuffer, Rgba};
use std::io::Write;
use termcolor::{BufferedStandardStream, Color, ColorChoice, ColorSpec, WriteColor};

use crossterm::cursor::MoveRight;
use crossterm::execute;
use crate::printer::block::masks::{choose_mask_and_colors, get_all_masks, SUBPIXEL64_COLUMNS, SUBPIXEL64_ROWS};


const UPPER_HALF_BLOCK: &str = "\u{2580}";
const UPPER_HALF_BLOCK_CHAR: char = '\u{2580}';
const LOWER_HALF_BLOCK: &str = "\u{2584}";
const LOWER_HALF_BLOCK_CHAR: char = '\u{2584}';

const CHECKERBOARD_BACKGROUND_LIGHT: (u8, u8, u8) = (153, 153, 153);
const CHECKERBOARD_BACKGROUND_DARK: (u8, u8, u8) = (102, 102, 102);

pub struct BlockPrinter;

impl Printer for BlockPrinter {
    fn print(
        &self,
        // TODO: The provided object is not used because termcolor needs an implementation of the WriteColor trait
        _stdout: &mut impl Write,
        img: &DynamicImage,
        config: &Config,
    ) -> ViuResult<(u32, u32)> {
        let mut stream = BufferedStandardStream::stdout(ColorChoice::Always);
        if config.sub_blocks {
            print_with_add_blocks(&mut stream, img, config)
        } else {
            print_to_writecolor(&mut stream, img, config)
        }
    }
}

fn print_with_add_blocks(
    stdout: &mut impl WriteColor,
    img: &DynamicImage,
    config: &Config
) -> ViuResult<(u32, u32)> {

    // resize the image so that it fits in the constraints, if any
    let img8 = super::resize8(img, config.width, config.height);
    let (s_width, s_height) = img8.dimensions();

    let img1 = super::resize(img, config.width, config.height);
    let (width, height) = img1.dimensions();

    if (s_width, s_height) != (width * SUBPIXEL64_COLUMNS, height * SUBPIXEL64_COLUMNS) {
        panic!("unable to scale image properly");
        // return print_to_writecolor(stdout, img, config);
    }

    // adjust with x=0 and handle horizontal offset entirely below
    adjust_offset(stdout, &Config { x: 0, ..*config })?;

    let img1_buffer = img1.to_rgba8();
    let img8_buffer = img8.to_rgba8();

    let get_color = |(row,col,color)| if is_pixel_transparent((row,col,color)) {
        if config.transparent {
            None
        } else {
            Some(transparency_color(row, col, config.truecolor))
        }
    } else {
        Some(if config.truecolor {
            Color::Rgb(color[0], color[1], color[2])
        } else {
            Color::Ansi256(ansi256_from_rgb((color[0], color[1], color[2])))
        })
    };
    let mask_cache = get_all_masks();

    for y in (0..height).step_by(2) {
        let is_last_row = y == height - 1;
        if config.x > 0 {
            execute!(stdout, MoveRight(config.x))?;
        }
        for x in 0..width {
            let mut write_default = true;
            if !is_last_row {
                if let Some((mask, fg_col, bg_col)) = choose_mask_and_colors(&mask_cache, (x * SUBPIXEL64_COLUMNS, y * SUBPIXEL64_COLUMNS), &img8_buffer, config) {
                    let mut colorspec = ColorSpec::new();
                    match (fg_col, bg_col) {
                        (None, Some(bg)) => {
                            colorspec.set_fg(Some(bg));
                            colorspec.set_bg(None);
                        }
                        _ => {
                            colorspec.set_bg(bg_col);
                            colorspec.set_fg(fg_col);
                        }
                    }
                    write_custom_colored_character(stdout, &colorspec, is_last_row, mask.char)?;
                    write_default = false;
                }
            }
            if write_default {
                if is_last_row {
                    let mut colorspec = ColorSpec::new();
                    let top = img1_buffer.get_pixel(x, y);
                    colorspec.set_bg(get_color((y, x, top)));
                    write_colored_character(stdout, &colorspec, is_last_row)?;
                } else {
                    let mut colorspec = ColorSpec::new();
                    let top = img1_buffer.get_pixel(x, y);
                    let bottom = img1_buffer.get_pixel(x, y + 1);
                    colorspec.set_bg(get_color((y, x, top)));
                    colorspec.set_fg(get_color((y, x, bottom)));
                    write_colored_character(stdout, &colorspec, is_last_row)?;
                }
            }
        }
        stdout.reset()?;
        writeln!(stdout, "\r")?;
    }
    stdout.reset()?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok((width, height / 2 + height % 2))
}

fn print_to_writecolor(
    stdout: &mut impl WriteColor,
    img: &DynamicImage,
    config: &Config,
) -> ViuResult<(u32, u32)> {
    // adjust with x=0 and handle horizontal offset entirely below
    adjust_offset(stdout, &Config { x: 0, ..*config })?;

    // resize the image so that it fits in the constraints, if any
    let img = super::resize(img, config.width, config.height);
    let (width, height) = img.dimensions();

    let mut row_color_buffer: Vec<ColorSpec> = vec![ColorSpec::new(); width as usize];
    let img_buffer = img.to_rgba8(); //TODO: Can conversion be avoided?

    for (curr_row, img_row) in img_buffer.enumerate_rows() {
        let is_even_row = curr_row % 2 == 0;
        let is_last_row = curr_row == height - 1;

        // move right if x offset is specified
        if config.x > 0 && (!is_even_row || is_last_row) {
            execute!(stdout, MoveRight(config.x))?;
        }

        for pixel in img_row {
            // choose the half block's color
            let color = if is_pixel_transparent(pixel) {
                if config.transparent {
                    None
                } else {
                    Some(transparency_color(curr_row, pixel.0, config.truecolor))
                }
            } else {
                Some(color_from_pixel(curr_row, pixel, config))
            };

            // Even rows modify the background, odd rows the foreground
            // because lower half blocks are used by default
            let colorspec = &mut row_color_buffer[pixel.0 as usize];
            if is_even_row {
                colorspec.set_bg(color);
                if is_last_row {
                    write_colored_character(stdout, colorspec, true)?;
                }
            } else {
                colorspec.set_fg(color);
                write_colored_character(stdout, colorspec, false)?;
            }
        }

        if !is_even_row && !is_last_row {
            stdout.reset()?;
            writeln!(stdout, "\r")?;
        }
    }

    stdout.reset()?;
    writeln!(stdout)?;
    stdout.flush()?;

    Ok((width, height / 2 + height % 2))
}

fn write_colored_character(
    stdout: &mut impl WriteColor,
    c: &ColorSpec,
    is_last_row: bool,
) -> ViuResult {
    write_custom_colored_character(stdout, c, is_last_row, LOWER_HALF_BLOCK_CHAR)
}
fn write_custom_colored_character(
    stdout: &mut impl WriteColor,
    c: &ColorSpec,
    is_last_row: bool,
    character: char,
) -> ViuResult {
    let out_color;
    let mut new_color;
    let mut out_char;
    // On the last row use upper blocks and leave the bottom half empty (transparent)
    if is_last_row {
        new_color = ColorSpec::new();
        if let Some(bg) = c.bg() {
            new_color.set_fg(Some(*bg));
            out_char = UPPER_HALF_BLOCK_CHAR;
        } else {
            execute!(stdout, MoveRight(1))?;
            return Ok(());
        }
        out_color = &new_color;
    } else {
        match (c.fg(), c.bg()) {
            (None, None) => {
                // completely transparent
                execute!(stdout, MoveRight(1))?;
                return Ok(());
            }
            (Some(bottom), None) => {
                // only top transparent
                new_color = ColorSpec::new();
                new_color.set_fg(Some(*bottom));
                out_color = &new_color;
                out_char = LOWER_HALF_BLOCK_CHAR;
            }
            (None, Some(top)) => {
                // only bottom transparent
                new_color = ColorSpec::new();
                new_color.set_fg(Some(*top));
                out_color = &new_color;
                out_char = UPPER_HALF_BLOCK_CHAR;
            }
            (Some(_top), Some(_bottom)) => {
                // both parts have a color
                out_color = c;
                out_char = character;
            }
        }
    }
    stdout.set_color(out_color)?;
    write!(stdout, "{}", out_char)?;

    Ok(())
}

fn is_pixel_transparent(pixel: (u32, u32, &Rgba<u8>)) -> bool {
    pixel.2[3] == 0
}

#[inline(always)]
fn checkerboard(row: u32, col: u32) -> (u8, u8, u8) {
    if row % 2 == col % 2 {
        CHECKERBOARD_BACKGROUND_DARK
    } else {
        CHECKERBOARD_BACKGROUND_LIGHT
    }
}

#[inline(always)]
fn transparency_color(row: u32, col: u32, truecolor: bool) -> Color {
    //imitate the transparent chess board pattern
    let rgb = checkerboard(row, col);
    if truecolor {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    } else {
        Color::Ansi256(ansi256_from_rgb(rgb))
    }
}

/// Composes the foreground over the background.
///
/// This assumes unpremultiplied alpha.
#[inline(always)]
fn over(fg: u8, bg: u8, alpha: u8) -> u8 {
    ((fg as u16 * alpha as u16 + bg as u16 * (255u16 - alpha as u16)) / 255) as _
}

/// Composes the foreground over the background.
///
/// This assumes premultiplied alpha (standard Porter-Duff compositing).
#[inline(always)]
fn over_porter_duff(fg: u8, bg: u8, alpha: u8) -> u8 {
    ((fg as u16 + bg as u16 * (255u16 - alpha as u16)) / 255) as _
}

#[inline(always)]
fn color_from_pixel(row: u32, pixel: (u32, u32, &Rgba<u8>), config: &Config) -> Color {
    let (col, _y, color) = pixel;
    let alpha = color[3];

    let rgb = if !config.transparent && alpha < 255 {
        // We need to blend the pixel's color with the checkerboard pattern.
        let checker = checkerboard(row, col);

        if config.premultiplied_alpha {
            (
                over_porter_duff(color[0], checker.0, alpha),
                over_porter_duff(color[1], checker.1, alpha),
                over_porter_duff(color[2], checker.2, alpha),
            )
        } else {
            (
                over(color[0], checker.0, alpha),
                over(color[1], checker.1, alpha),
                over(color[2], checker.2, alpha),
            )
        }
    } else {
        (color[0], color[1], color[2])
    };

    if config.truecolor {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    } else {
        Color::Ansi256(ansi256_from_rgb(rgb))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::*;
    use termcolor::{Ansi, Color};
    // Note: truecolor is not supported in CI. Hence, it should be disabled when writing the tests

    #[test]
    fn test_block_printer_e2e() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::new(5, 4));
        let mut buf = Ansi::new(vec![]);

        let config = Config {
            truecolor: false,
            ..Default::default()
        };

        let (w, h) = print_to_writecolor(&mut buf, &img, &config).unwrap();
        assert_eq!((w, h), (5, 2));

        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[1;1H\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\r\n\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\n"
        );
    }

    #[test]
    fn test_block_printer_e2e_transparent() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::new(5, 4));
        let mut buf = Ansi::new(vec![]);

        let config = Config {
            transparent: true,
            ..Default::default()
        };

        let (w, h) = print_to_writecolor(&mut buf, &img, &config).unwrap();
        assert_eq!((w, h), (5, 2));

        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[1;1H\x1b[1C\x1b[1C\x1b[1C\x1b[1C\x1b[1C\x1b[0m\r\n\x1b[1C\x1b[1C\x1b[1C\x1b[1C\x1b[1C\x1b[0m\n"
        );
    }

    #[test]
    fn test_block_printer_e2e_odd_height() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::new(4, 3));
        let mut buf = Ansi::new(vec![]);

        let config = Config {
            truecolor: false,
            absolute_offset: false,
            ..Default::default()
        };
        let (w, h) = print_to_writecolor(&mut buf, &img, &config).unwrap();
        assert_eq!((w, h), (4, 2));

        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\x1b[38;5;247m\x1b[48;5;241m▄\x1b[0m\x1b[38;5;241m\x1b[48;5;247m▄\x1b[0m\r\n\x1b[0m\x1b[38;5;241m▀\x1b[0m\x1b[38;5;247m▀\x1b[0m\x1b[38;5;241m▀\x1b[0m\x1b[38;5;247m▀\x1b[0m\n"
        );
    }

    #[test]
    fn test_write_colored_char_only_fg() {
        let mut buf = Ansi::new(vec![]);
        let mut c = ColorSpec::new();

        c.set_fg(Some(Color::Rgb(10, 20, 30)));

        write_colored_character(&mut buf, &c, false).unwrap();
        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[0m\x1b[38;2;10;20;30m▄"
        );
    }

    #[test]
    fn test_write_colored_char_only_bg() {
        let mut buf = Ansi::new(vec![]);
        let mut c = ColorSpec::new();

        c.set_bg(Some(Color::Rgb(50, 60, 70)));

        write_colored_character(&mut buf, &c, false).unwrap();
        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[0m\x1b[38;2;50;60;70m▀"
        );
    }

    #[test]
    fn test_write_colored_char_fg_and_bg() {
        let mut buf = Ansi::new(vec![]);
        let mut c = ColorSpec::new();

        c.set_fg(Some(Color::Rgb(10, 20, 30)));
        c.set_bg(Some(Color::Rgb(15, 25, 35)));

        write_colored_character(&mut buf, &c, false).unwrap();
        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[0m\x1b[38;2;10;20;30m\x1b[48;2;15;25;35m▄"
        );
    }

    #[test]
    fn test_write_colored_char_no_color() {
        let mut buf = Ansi::new(vec![]);
        let c = ColorSpec::new();

        write_colored_character(&mut buf, &c, false).unwrap();
        // expect to print nothing, just move cursor to the right
        assert_eq!(std::str::from_utf8(buf.get_ref()).unwrap(), "\x1b[1C");
    }

    #[test]
    fn test_write_colored_char_last_row_bg() {
        let mut buf = Ansi::new(vec![]);
        let mut c = ColorSpec::new();

        c.set_bg(Some(Color::Rgb(10, 20, 30)));

        write_colored_character(&mut buf, &c, true).unwrap();
        assert_eq!(
            std::str::from_utf8(buf.get_ref()).unwrap(),
            "\x1b[0m\x1b[38;2;10;20;30m▀"
        );
    }

    #[test]
    fn test_write_colored_char_last_row_no_bg() {
        let mut buf = Ansi::new(vec![]);
        let mut c = ColorSpec::new();

        // test with no color
        write_colored_character(&mut buf, &c, true).unwrap();
        assert_eq!(std::str::from_utf8(buf.get_ref()).unwrap(), "\x1b[1C");

        c.set_fg(Some(Color::Rgb(10, 20, 30)));

        // test with fg (unusual case)
        let mut buf = Ansi::new(vec![]);
        write_colored_character(&mut buf, &c, true).unwrap();
        assert_eq!(std::str::from_utf8(buf.get_ref()).unwrap(), "\x1b[1C");
    }

    // test print the smiley
    // cargo test --color=always --package viuer --lib printer::block::tests --no-fail-fast
    #[test]
    fn test_write_img() {
        let make_conf = |x, y, width, height, sub_blocks: bool, t: bool| Config {
            x, y, sub_blocks, transparent: t,
            width: Some(width), height: Some(height),
            ..Default::default()
        };
        let height = 30;
        let width = 2 * height;
        let a_conf = make_conf(0, 0, width, height, false, false);
        let b_conf = make_conf(width as u16, 0, width, height, true, false);
        let img = image::ImageReader::open("pancake.png").unwrap()
            .with_guessed_format().unwrap()
            .decode().unwrap();
        let mut stream = BufferedStandardStream::stdout(ColorChoice::Always);
        BlockPrinter.print(&mut stream, &img, &a_conf).expect("Image printing failed.");
        BlockPrinter.print(&mut stream, &img, &b_conf).expect("Image printing failed.");
    }
}
