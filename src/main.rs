mod pulseaudio;
use core::panic;

use image::{GenericImageView, ImageBuffer, Rgb, open};

use rayon::prelude::*;

use std::{
    arch::x86_64::*,
    char::from_u32,
    env,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write, stdout},
    path::PathBuf,
    thread,
    time::{self, Duration},
};
use std::fs::FileType;
use zstd::{
    Encoder,
    dict::{DecoderDictionary, EncoderDictionary},
    stream::Decoder,
};

use image::imageops::FilterType;
use std::os::unix;
use terminal_size::{terminal_size, Height, Width};

fn mkdir(dir_name: &str) {
    match fs::create_dir(dir_name) {
        Err(e) => println!("{}: {}", dir_name, e.kind()),
        Ok(_) => {}
    }
}

fn get_pixelcolor(rgb: &u8) -> bool {
    if rgb < &148 { false } else { true }
}

fn get_pixel(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, x: &u32, y: &u32) -> u32 {
    if img.width() > *x && img.height() > *y {
        get_pixelcolor(&img.get_pixel(*x, *y)[0]) as u32
    } else {
        0
    }
}

fn braille_buf(img: &Vec<Vec<bool>>, width: &u32, height: &u32) -> String {
    //2*size.unwrap().0.0 as u32,4*(size.unwrap().1.0 -1) as u32
    let positions = [
        (0, 0, 0),
        (0, 1, 1),
        (0, 2, 2),
        (1, 0, 3),
        (1, 1, 4),
        (1, 2, 5),
        (0, 3, 6),
        (1, 3, 7),
    ];

    let src_height = img.len() as u32;
    let src_width = img[0].len() as u32;

    let rows: Vec<u32> = (0..height / 4).collect();
    let lines: Vec<String> = rows
        .into_par_iter()
        .map(|py| {
            let mut codes = Vec::with_capacity((width / 2) as usize);
            for px in (0..width / 2) {
                let mut colors = 0u32;

                for &(dx, dy, bit) in &positions {
                    let x = px*2 + dx;
                    let y = py*4 + dy;

                    let src_x = x * src_width / width;
                    let src_y = y * src_height /  height;

                    if src_x < src_width && src_y < src_height {
                        if img[src_y as usize][src_x as usize] {
                            colors |= 1 << bit;
                        }
                    }
                }

                codes.push(0x2800 + colors);
            }
            unsafe { String::from_iter(codes.iter().map(|&c| char::from_u32_unchecked(c))) }
        })
        .collect();
    lines.join("\n")
}

fn braille_iter<'a>(img: &'a Resized<'a>) -> String {
    //2*size.unwrap().0.0 as u32,4*(size.unwrap().1.0 -1) as u32
    let positions = [
        (0, 0, 0),
        (0, 1, 1),
        (0, 2, 2),
        (1, 0, 3),
        (1, 1, 4),
        (1, 2, 5),
        (0, 3, 6),
        (1, 3, 7),
    ];
    let rows: Vec<u32> = (0..img.height).step_by(4).collect();

    let mut lines: Vec<String> = rows
        .into_par_iter()
        .map(|py| {
            let mut codes = Vec::with_capacity((img.width / 2) as usize);
            for px in (0..img.width).step_by(2) {
                let mut colors = 0u32;
                for &(dx, dy, bit) in &positions {
                    let x = px + dx;
                    let y = py + dy;
                    if x < img.width && y < img.height {
                        if img.read_rgb(x, y) {
                            colors |= 1 << bit;
                        }
                    }
                }

                codes.push(0x2800 + colors);
            }
            unsafe { String::from_iter(codes.iter().map(|&c| char::from_u32_unchecked(c))) }
        })
        .collect();
    lines.join("\n")
}

struct FastImage<'a> {
    buf: &'a [u8],
    width: u32,
    height: u32,
}

struct Resized<'a> {
    src: &'a FastImage<'a>,
    width: u32,
    height: u32,
}

impl<'a> FastImage<'a> {
    fn from_image_buffer(img: &'a ImageBuffer<Rgb<u8>, Vec<u8>>) -> Self {
        FastImage {
            buf: img.as_raw(),
            width: img.width(),
            height: img.height(),
        }
    }
    fn from_buffer(buf: &'a Vec<u8>, width: u32, height: u32) -> Self {
        FastImage {
            buf: buf,
            width: width,
            height: height,
        }
    }
    fn resize_nearest<'b>(&'b self, x: u16, y: u16) -> Resized<'b> {
        Resized {
            src: self,
            width: x as u32,
            height: y as u32,
        }
    }
    fn resize_asp(&self, target_width: u16, target_height: u16) -> Resized<'_> {
        let char_aspect = 4.0 / 2.0;
        let target_px_w = target_width * 2;
        let target_px_h = target_height * 4;

        let aspect = self.width as f32 / self.height as f32;
        let target_aspect = (target_px_w as f32 / target_px_h as f32) * char_aspect;

        let (new_w, new_h) = if aspect > target_aspect {
            (
                target_px_w,
                ((target_px_w as f32 / aspect).ceil() as u16).min(target_px_h),
            )
        } else {
            (
                ((target_px_h as f32 * aspect).ceil() as u16).min(target_px_w),
                target_px_h,
            )
        };
        self.resize_nearest(new_w, new_h)
    }
}

impl<'a> Resized<'a> {
    #[inline(always)]
    fn read_rgb(&self, x: u32, y: u32) -> bool {
        let src_x = x * self.src.width / self.width;
        let src_y = y * self.src.height / self.height;

        let idx = ((src_y * self.src.width + src_x) * 3) as usize;
        if idx < self.src.buf.len() {
            //self.src.buf[idx]
            ((0.299 * self.src.buf[idx] as f32
                + 0.587 * self.src.buf[idx + 1] as f32
                + 0.114 * self.src.buf[idx + 2] as f32)
                .round() as u8)
                > 128u8
        } else {
            false
        }
    }
}

fn to_ascii<'a>(img: &'a Resized<'a>) -> io::Result<()> {
    print!("{}", &braille_iter(img));

    Ok(())
}

fn time_formatter(time: u16) -> String {
    return format!("{}m{}s", time / 60, time % 60);
}

fn compress(file_name: &str, max_frame: &u16) -> io::Result<()> {
    let now = time::Instant::now();

    let output = File::create(file_name)?;
    let writer = BufWriter::new(output);

    let dict_data = std::fs::read("dict.zstd")?;

    let mut encoder = Encoder::with_dictionary(writer, 19, &dict_data)?;

    fn bool_to_u8(bits: &[bool]) -> Vec<u8> {
        let mut vec = Vec::new();
        let mut value = 0u8;

        let mut loopcount = 0u8;
        for &bit in bits {
            if bit {
                value += 1;
            }
            if 6 < loopcount {
                vec.push(value);
                value = 0;
                loopcount = 0;
            } else {
                value <<= 1;
                loopcount += 1;
            }
        }

        vec
    }

    for frame_count in 1..=(!max_frame as usize) {
        let frame: PathBuf = format!("output/frame/{frame_count}.png").into();
        let frame_bin: PathBuf = format!("output/frame_bin/{frame_count}").into();
        println!("{:?}", frame);

        if frame.exists() {
            let raw = open(&frame).unwrap();
            let image = raw.as_rgb8().unwrap();

            let mut vec = Vec::new();
            for y in 0..image.height() {
                for x in 0..image.width() {
                    vec.push(image.get_pixel(x, y)[0] < 128);
                }
            }

            let image_buffer = &bool_to_u8(&vec);

            if frame_count == 1 {
                encoder.write_all(&image.width().to_le_bytes())?;
                encoder.write_all(&image.height().to_le_bytes())?;

                encoder.write_all(&(image_buffer.len() as u32).to_le_bytes())?;
            }

            encoder.write_all(image_buffer)?;
            //File::create(frame_bin)?.write_all(image_buffer)?;
        } else {
            println!("Error file {:?} is not Exists.", frame);
        }
    }
    encoder.finish()?;
    println!("{:?}", now.elapsed());

    Ok(()) //exit status
}

fn decode(file_name: &str) -> io::Result<()> {
    fn resize_asp(width: u32, height: u32, target_width: u32, target_height: u32) -> (u32, u32) {
        if width == target_width && height == target_height {
            return (width, height);
        }

        let char_aspect = 4.0 / 2.0;
        let target_px_w = target_width * 2;
        let target_px_h = target_height * 4;

        let aspect = width as f32 / height as f32;
        let target_aspect = (target_px_w as f32 / target_px_h as f32) * char_aspect;

        let (new_w, new_h ) = if aspect > target_aspect {
            (
                target_px_w,
                ((target_px_w as f32 / aspect).ceil() as u32).min(target_px_h),
            )
        } else {
            (
                ((target_px_h as f32 * aspect).ceil() as u32).min(target_px_w),
                target_px_h,
            )
        };
        (new_w, new_h)
    };

    let dict_data = std::fs::read("dict.zstd")?;
    let dict = DecoderDictionary::copy(&dict_data);

    let reader = BufReader::new(File::open(file_name)?);
    let mut decoder = Decoder::with_dictionary(reader, &dict_data)?;

    print!("\x1B[?25l"); //HideCursor

    let mut total_process_duration = Duration::from_secs(0);
    let play_started_time = time::Instant::now();
    let frame_delay: u64 = 1000_000_000 / 60;
    let mut played_frame: u64 = 0;

    thread::spawn(|| {
        pulseaudio::play_audio();
    });

    let mut dimension_buf_len = [0u8; 4];
    decoder.read_exact(&mut dimension_buf_len)?;
    let width = u32::from_le_bytes(dimension_buf_len) as u32;

    decoder.read_exact(&mut dimension_buf_len)?;
    let height = u32::from_le_bytes(dimension_buf_len) as u32;

    let mut len_buf = [0u8; 4];

    decoder.read_exact(&mut len_buf)?;
    let frame_len = u32::from_le_bytes(len_buf) as u32;
    let mut frame = 0;
    loop {
        frame += 1;
        let mut frame_buf = vec![0u8; (frame_len) as usize];
        if decoder.read_exact(&mut frame_buf).is_err() {
            println!("==== END ====");
            break;
        };
        //let frame = u32::from_le_bytes(frame_buf) as u32;

        let now = time::Instant::now();

        let (twidth, theight) = {
            let term = terminal_size().unwrap();
            (term.0.0 as u32, term.1.0 as u32)
        };

        //print!("\x1B[0;0H");
        // ASCII is here!

            let mut vec: Vec<Vec<bool>> = Vec::new();

            let mut vec2 = Vec::new();
            frame_buf.iter().enumerate().for_each(|(index, f)| {
                if index as u32 % (width / 8) == 0 && index != 0{
                    vec.push(vec2.clone());
                    vec2 = Vec::new();
                }

                for i in 0..8 {
                    vec2.push((f >> (7-i) & 1u8) == 0u8);
                }
            });



        //let string: String = vec.iter().map(|x| {x.iter().map(|b| {if *b { '1' } else { '0' }}).collect::<String>()} + "\n").collect();
        let (w, h) = {
            let w = if twidth < width {twidth - 1} else {width};
            let h = if theight < height {theight - 1} else {height};

            resize_asp(width, height, w, h)
        };

        print!("\x1B[0;0H");

        println!("{}", braille_buf(&vec, &w, &h));

        // 処理時間などの統計表示.
        played_frame += frame_delay;
        let duration = now.elapsed();
        print!("\x1B[0;0H");
        print!(
            "{:>7}frame | dul: {:?}",
            frame,
            duration,
        );
        stdout().flush()?;
        total_process_duration += now.elapsed();
        thread::sleep(time::Duration::from_nanos(
            frame_delay.saturating_sub(
                play_started_time
                    .elapsed()
                    .as_nanos()
                    .saturating_sub(played_frame as u128) as u64,
            ),
        ));
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    //loop {
    let args: Vec<String> = env::args().collect();

    let frame_delay: u64 = 1000_000_000 / 60;
    let max_frame = fs::read_dir("output/frame")?.count() as u16;
    let max_time = time_formatter(max_frame as u16 / 60);

    if args.iter().count() != 1 {
        let file_name = "movie.zst";
        match &args[1] as &str {
            "--encode" => compress(&file_name, &max_frame),
            "--decode" => decode(&file_name),
            _ => Ok(()),
        }?;
    } else {
        let mut played_frame: u64 = 0;
        thread::spawn(|| {
            pulseaudio::play_audio();
        });

        print!("\x1B[?25l"); //HideCursor

        let mut total_process_duration = Duration::from_secs(0);
        let play_started_time = time::Instant::now();
        //for (i, frame) in frame_dir.into_iter().enumerate() {
        //
        for frame_count in 1..=max_frame as usize {
            let now = time::Instant::now();

            let frame = format!("output/frame/{frame_count}.png");

            let size = terminal_size().unwrap();
            let image = open(&frame).unwrap().to_rgb8();
            let fast_image = FastImage::from_image_buffer(&image);
            print!("\x1B[0;0H");
            let _ = to_ascii(&fast_image.resize_asp(size.0.0, size.1.0));

            played_frame += frame_delay;
            let duration = now.elapsed();
            print!("\x1B[0;0H");
            print!(
                "{}/{:<5} | now/end: {:>6}/{:<6} | progress: {:.3}% | processDuration: {:?}",
                frame,
                max_frame,
                time_formatter(frame_count as u16 / 60),
                max_time,
                frame_count as f32 / max_frame as f32 * 100.0,
                duration,
            );
            stdout().flush().unwrap();
            total_process_duration += now.elapsed();
            thread::sleep(time::Duration::from_nanos(
                frame_delay.saturating_sub(
                    play_started_time
                        .elapsed()
                        .as_nanos()
                        .saturating_sub(played_frame as u128) as u64,
                ),
            ));
        }
        println!(
            "averageProcessTime: {:?}",
            total_process_duration / max_frame as u32
        );
    }
    Ok(())
    //}
}
