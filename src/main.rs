mod pulseaudio;

use image::open;

use rayon::prelude::*;

use std::{
    env,
    fs::{self, File},
    io::{self, stdout, BufReader, BufWriter, Read, Write},
    path::PathBuf,
    thread,
    time::{self, Duration},
};
use terminal_size::terminal_size;
use zstd::{
    stream::Decoder,
    Encoder,
};

fn braille_buf(img: &Vec<Vec<bool>>, width: &u32, height: &u32) -> String {
    let positions = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];

    let src_height = img.len() as u32;
    let src_width = img[0].len() as u32;

    let rows: Vec<u32> = (0..height >> 2).collect();
    let lines: Vec<String> = rows
        .into_par_iter()
        .map(|py| {
            let mut codes = Vec::with_capacity((width >> 1) as usize);
            for px in 0..width >> 1 {
                let mut colors = 0u32;

                for (index, &(dx, dy)) in positions.iter().enumerate() {
                    let x = (px << 1) + dx;
                    let y = (py << 2) + dy;

                    let src_x = x * src_width / width;
                    let src_y = y * src_height / height;

                    if src_x < src_width && src_y < src_height {
                        if img[src_y as usize][src_x as usize] {
                            colors |= 1 << index;
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

fn compress(file_name: &str, max_frame: &u16) -> io::Result<()> {
    let now = time::Instant::now();

    let output = File::create(file_name)?;
    let writer = BufWriter::new(output);

    let dict_data = fs::read("dict.zstd")?;

    let mut encoder = Encoder::with_dictionary(writer, 19, &dict_data)?;

    fn bool_to_u8(bits: &[bool]) -> Vec<u8> {
        let mut vec = Vec::new();
        let mut value = 0u8;

        let mut loop_count = 0u8;
        for &bit in bits {
            if bit {
                value += 1;
            }
            if 6 < loop_count {
                vec.push(value);
                value = 0;
                loop_count = 0;
            } else {
                value <<= 1;
                loop_count += 1;
            }
        }

        vec
    }

    for frame_count in 1..=(!max_frame as usize) {
        let frame: PathBuf = format!("output/frame/{frame_count}.png").into();
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

        let (new_w, new_h) = if aspect > target_aspect {
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
    }

    let dict_data = fs::read("dict.zstd")?;

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
    let width = u32::from_le_bytes(dimension_buf_len);

    decoder.read_exact(&mut dimension_buf_len)?;
    let height = u32::from_le_bytes(dimension_buf_len);

    let mut len_buf = [0u8; 4];

    decoder.read_exact(&mut len_buf)?;
    let frame_len = u32::from_le_bytes(len_buf);
    let mut frame = 0;
    loop {
        let now = time::Instant::now();
        frame += 1;

        //TODO: if generating frame is delayed, to skip generating.
        let ascii = if true {
            let mut frame_buf = vec![0u8; frame_len as usize];
            if decoder.read_exact(&mut frame_buf).is_err() {
                println!("==== END ====");
                break;
            };

            let (terminal_width, terminal_height) = {
                let term = terminal_size().unwrap();
                (term.0.0 as u32, term.1.0 as u32)
            };

            let mut vec: Vec<Vec<bool>> = Vec::new();

            let mut vec2 = Vec::new();
            frame_buf.iter().enumerate().for_each(|(index, f)| {
                if index as u32 % (width / 8) == 0 && index != 0 {
                    vec.push(vec2.clone());
                    vec2 = Vec::new();
                }

                for i in 0..8 {
                    vec2.push((f >> (7 - i) & 1u8) == 0u8);
                }
            });

            //let string: String = vec.iter().map(|x| {x.iter().map(|b| {if *b { '1' } else { '0' }}).collect::<String>()} + "\n").collect();
            let (w, h) = {
                let w = if terminal_width < width { terminal_width - 1 } else { width };
                let h = if terminal_height < height { terminal_height - 1 } else { height };

                resize_asp(width, height, w, h)
            };

            braille_buf(&vec, &w, &h)
        } else { String::new() };

        print!("\x1B[0;0H");

        if frame % 60 == 0 {
            //clear screen every 60 frames(1 second)
            println!("\x1b[2J");
        }
        println!("{}", ascii);

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
        thread::sleep(Duration::from_nanos(
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

    let max_frame = fs::read_dir("output/frame")?.count() as u16;

    let file_name = "movie.zst";
    if args.iter().count() != 1 {
        match &args[1] as &str {
            "--encode" => compress(&file_name, &max_frame),
            "--decode" => decode(&file_name),
            _ => Ok(()),
        }?
    } else {
        decode(&file_name)?;
    }
    Ok(())
}
