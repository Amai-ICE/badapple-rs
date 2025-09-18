use libpulse_simple_binding::Simple;
use symphonia::core::{
    audio::{Channels, RawSampleBuffer, SignalSpec},
    codecs::{CODEC_TYPE_NULL, DecoderOptions},
    errors::Error,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

// Symphonia https://github.com/pdeljanov/Symphonia/blob/master/symphonia-play/src/
// MIT-License
//---------------------------------------------------------------------------------------------------------------
fn play_sample(spec: SignalSpec) -> Box<Simple> {
    let pa_spec = libpulse_binding::sample::Spec {
        format: libpulse_binding::sample::Format::FLOAT32NE,
        channels: spec.channels.count() as u8,
        rate: spec.rate,
    };
    let pa_ch_map = map_channels_to_pa_channelmap(spec.channels);

    let pulse_audio = libpulse_simple_binding::Simple::new(
        None,
        "BadApple",
        libpulse_binding::stream::Direction::Playback,
        None,
        "Music",
        &pa_spec,
        pa_ch_map.as_ref(),
        None,
    );

    match pulse_audio {
        Ok(pa) => Box::new(pa),
        Err(e) => panic!("{}", e),
    }
}
fn map_channels_to_pa_channelmap(channels: Channels) -> Option<libpulse_binding::channelmap::Map> {
    let mut map: libpulse_binding::channelmap::Map = Default::default();
    map.init();
    map.set_len(channels.count() as u8);

    let is_mono = channels.count() == 1;

    for (i, channel) in channels.iter().enumerate() {
        map.get_mut()[i] = match channel {
            Channels::FRONT_LEFT if is_mono => libpulse_binding::channelmap::Position::Mono,
            Channels::FRONT_LEFT => libpulse_binding::channelmap::Position::FrontLeft,
            Channels::FRONT_RIGHT => libpulse_binding::channelmap::Position::FrontRight,
            Channels::FRONT_CENTRE => libpulse_binding::channelmap::Position::FrontCenter,
            Channels::REAR_LEFT => libpulse_binding::channelmap::Position::RearLeft,
            Channels::REAR_CENTRE => libpulse_binding::channelmap::Position::RearCenter,
            Channels::REAR_RIGHT => libpulse_binding::channelmap::Position::RearRight,
            Channels::LFE1 => libpulse_binding::channelmap::Position::Lfe,
            Channels::FRONT_LEFT_CENTRE => {
                libpulse_binding::channelmap::Position::FrontLeftOfCenter
            }
            Channels::FRONT_RIGHT_CENTRE => {
                libpulse_binding::channelmap::Position::FrontRightOfCenter
            }
            Channels::SIDE_LEFT => libpulse_binding::channelmap::Position::SideLeft,
            Channels::SIDE_RIGHT => libpulse_binding::channelmap::Position::SideRight,
            Channels::TOP_CENTRE => libpulse_binding::channelmap::Position::TopCenter,
            Channels::TOP_FRONT_LEFT => libpulse_binding::channelmap::Position::TopFrontLeft,
            Channels::TOP_FRONT_CENTRE => libpulse_binding::channelmap::Position::TopFrontCenter,
            Channels::TOP_FRONT_RIGHT => libpulse_binding::channelmap::Position::TopFrontRight,
            Channels::TOP_REAR_LEFT => libpulse_binding::channelmap::Position::TopRearLeft,
            Channels::TOP_REAR_CENTRE => libpulse_binding::channelmap::Position::TopRearCenter,
            Channels::TOP_REAR_RIGHT => libpulse_binding::channelmap::Position::TopRearRight,
            _ => {
                // If a Symphonia channel cannot map to a PulseAudio position then return None
                // because PulseAudio will not be able to open a stream with invalid channels.

                return None;
            }
        }
    }

    Some(map)
}

pub fn play_audio() {
    let file = "output/audio/output.mp3";
    let source = std::fs::File::open(file).expect("Failed to open Media");
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let meta_opts: MetadataOptions = Default::default();
    let fmt_ops: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_ops, &meta_opts)
        .expect("unsported format");
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("no supported audio tracks");

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .expect("unsported codec");
    let track_id = track.id;

    let mut audio_output = None;
    let mut sample_buf = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(e) => panic!("{}", e),
        };
        while !format.metadata().is_latest() {
            format.metadata().pop();
        }

        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(_decoded) => {
                if audio_output.is_none() && sample_buf.is_none() {
                    let spec = *_decoded.spec();
                    let duration = _decoded.capacity() as u64;
                    audio_output.replace(play_sample(spec));
                    sample_buf.replace(RawSampleBuffer::<f32>::new(duration, spec));
                }

                if let Some(ref audio_output) = audio_output {
                    if let Some(ref mut buf) = sample_buf {
                        buf.copy_interleaved_ref(_decoded);
                        audio_output.write(buf.as_bytes()).unwrap();
                    }
                }
            }
            Err(Error::IoError(_)) => continue,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => panic!("{}", e),
        }
    }
}
//---------------------------------------------------------------------------------------------------------------
//END
