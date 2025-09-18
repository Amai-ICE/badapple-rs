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

use zstd::{
    Encoder,
    dict::{DecoderDictionary, EncoderDictionary},
    stream::Decoder,
};

use image::imageops::FilterType;
use std::os::unix;
use terminal_size::terminal_size;

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
                        let color = img.get_rgb(x, y);
                        if 128 < color {
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
    fn get_r(&self, x: u32, y: u32) -> u8 {
        let cx = x.min(self.width.saturating_sub(1));
        let cy = y.min(self.height.saturating_sub(1));

        let src_x =
            ((cx as f32 + 0.5) * (self.src.width as f32 / self.width as f32)).floor() as u32;
        let src_y =
            ((cy as f32 + 0.5) * (self.src.height as f32 / self.height as f32)).floor() as u32;

        let idx = ((src_y * self.src.width + src_x) * 3) as usize;
        self.src.buf[idx]
    }
    #[inline(always)]
    fn get_rgb(&self, x: u32, y: u32) -> u8 {
        let src_x = x * self.src.width / self.width;
        let src_y = y * self.src.height / self.height;

        let idx = ((src_y * self.src.width + src_x) * 3) as usize;
        if idx < self.src.buf.len() {
            //self.src.buf[idx]
            (0.299 * self.src.buf[idx] as f32
                + 0.587 * self.src.buf[idx + 1] as f32
                + 0.114 * self.src.buf[idx + 2] as f32)
                .round() as u8
        } else {
            0
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
    let mut writer = BufWriter::new(output);

    let dict_data = std::fs::read("dict.zstd")?;

    let mut encoder = Encoder::with_dictionary(writer, 19, &dict_data)?;

    for frame_count in 1..=(!max_frame as usize) {
        let frame: PathBuf = format!("output/frame/{frame_count}.png").into();
        println!("{:?}", frame);

        if frame.exists() {
            let size = terminal_size().unwrap();
            let image = open(&frame).unwrap().to_rgb8();
            let fast_image = FastImage::from_image_buffer(&image);

            if frame_count == 1 {
                encoder.write_all(&(fast_image.buf.len() as u64).to_le_bytes())?;
            }
            encoder.write_all(&fast_image.buf)?;
        } else {
            println!("Error file {:?} is not Exists.", frame);
        }
    }
    encoder.finish()?;
    println!("{:?}", now.elapsed());

    Ok(()) //exit status
}

fn decode(file_name: &str) -> io::Result<()> {
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

    let mut len_buf = [0u8; 8];

    decoder.read_exact(&mut len_buf)?;
    let frame_len = u64::from_le_bytes(len_buf) as usize;

    loop {
        let mut frame_buf = vec![0u8; frame_len];
        if decoder.read_exact(&mut frame_buf).is_err() {
            break;
        };

        let now = time::Instant::now();

        let size = terminal_size().unwrap();
        let fast_image = FastImage::from_buffer(&frame_buf, 480, 360);
        print!("\x1B[0;0H");
        let _ = to_ascii(&fast_image.resize_asp(size.0.0, size.1.0));

        /*
        match &open(&frame).unwrap().resize(2*size.unwrap().0.0 as u32,4*(size.unwrap().1.0 -1) as u32,FilterType::Nearest).as_rgb8() {
            None => Ok(()),
            Some(image) => to_ascii(&image),
        };*/

        played_frame += frame_delay;
        let duration = now.elapsed();
        print!("\x1B[0;0H");
        print!("{:?}", duration);
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

            /*
            match &open(&frame).unwrap().resize(2*size.unwrap().0.0 as u32,4*(size.unwrap().1.0 -1) as u32,FilterType::Nearest).as_rgb8() {
                None => Ok(()),
                Some(image) => to_ascii(&image),
            };*/

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
