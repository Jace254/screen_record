extern crate repng;
extern crate scrap;
extern crate serde;
extern crate webm;

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{io, thread};

use scrap::codec::{EncoderApi, EncoderCfg, Quality as Q};
use webm::mux;
use webm::mux::Track;

use scrap::vpxcodec as vpx_encode;
use scrap::{Capturer, Display, TraitCapturer, STRIDE_ALIGN};

// Default values
const DEFAULT_DURATION: u64 = 60; // seconds
const DEFAULT_FPS: u64 = 30;
const DEFAULT_QUALITY: Quality = Quality::Best;
const DEFAULT_CODEC: Codec = Codec::Vp9;
const OUTPUT_PATH: &str = "output.webm";

#[derive(Debug, serde::Deserialize)]
struct Args {
    arg_path: PathBuf,
    flag_codec: Codec,
    flag_time: Option<u64>,
    flag_fps: u64,
    flag_quality: Quality,
}

#[derive(Debug, serde::Deserialize)]
enum Quality {
    Best,
    Balanced,
    Low,
}

#[derive(Debug, serde::Deserialize)]
enum Codec {
    Vp8,
    Vp9,
}

fn main() -> io::Result<()> {
    // Setup arguments with default values
    let args = Args {
        arg_path: PathBuf::from(OUTPUT_PATH),
        flag_codec: DEFAULT_CODEC,
        flag_time: Some(DEFAULT_DURATION),
        flag_fps: DEFAULT_FPS,
        flag_quality: DEFAULT_QUALITY,
    };

    let duration = args.flag_time.map(Duration::from_secs);

    let d = Display::all().unwrap().remove(0);
    let (width, height) = (d.width() as u32, d.height() as u32);

    // Setup the multiplexer.
    let out = match OpenOptions::new().write(true).create_new(true).open(&args.arg_path) {
        Ok(file) => file,
        Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => {
            File::create(&args.arg_path)?
        }
        Err(e) => return Err(e.into()),
    };

    let mut webm =
        mux::Segment::new(mux::Writer::new(out)).expect("Could not initialize the multiplexer.");

    let (vpx_codec, mux_codec) = match args.flag_codec {
        Codec::Vp8 => (vpx_encode::VpxVideoCodecId::VP8, mux::VideoCodecId::VP8),
        Codec::Vp9 => (vpx_encode::VpxVideoCodecId::VP9, mux::VideoCodecId::VP9),
    };

    let mut vt = webm.add_video_track(width, height, None, mux_codec);

    // Setup the encoder.
    let quality = match args.flag_quality {
        Quality::Best => Q::Best,
        Quality::Balanced => Q::Balanced,
        Quality::Low => Q::Low,
    };
    let mut vpx = vpx_encode::VpxEncoder::new(
        EncoderCfg::VPX(vpx_encode::VpxEncoderConfig {
            width,
            height,
            quality,
            codec: vpx_codec,
            keyframe_interval: None,
        }),
        false,
    )
    .unwrap();

    // Start recording.
    let start = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));

    // To simulate user stopping the recording, use a separate thread.
    thread::spawn({
        let stop = stop.clone();
        move || {
            println!("Recording! Press Enter to stop.");
            let _ = std::io::stdin().read_line(&mut String::new());
            stop.store(true, Ordering::Release);
        }
    });

    let spf = Duration::from_nanos(1_000_000_000 / args.flag_fps);

    // Capturer object is expensive, avoiding to create it frequently.
    let mut c = Capturer::new(d).unwrap();
    let mut yuv = Vec::new();
    let mut mid_data = Vec::new();
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let time = now - start;

        if Some(true) == duration.map(|d| time > d) {
            break;
        }

        if let Ok(frame) = c.frame(Duration::from_millis(0)) {
            let ms = time.as_secs() * 1000 + time.subsec_millis() as u64;
            frame.to(vpx.yuvfmt(), &mut yuv, &mut mid_data).unwrap();
            for frame in vpx.encode(ms as i64, &yuv, STRIDE_ALIGN).unwrap() {
                vt.add_frame(frame.data, frame.pts as u64 * 1_000_000, frame.key);
            }
        }

        let dt = now.elapsed();
        if dt < spf {
            thread::sleep(spf - dt);
        }
    }

    // End things.
    let _ = webm.finalize(None);

    Ok(())
}
