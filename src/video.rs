use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use vid2img::FileSource;

fn create_output_directory(video_path: &str) -> Result<PathBuf, Box<dyn Error>> {
    let canonical_path = fs::canonicalize(video_path)?;
    let path_str = canonical_path
        .to_str()
        .ok_or("Invalid UTF-8 in video path")?;
    let video_hash = hex::encode(Sha256::digest(path_str.as_bytes()));
    let output_dir = std::env::temp_dir().join(format!("videohash_{}", video_hash));

    if !output_dir.exists() {
        fs::create_dir(&output_dir)?;
    }

    Ok(output_dir)
}
fn get_video_duration(video_path: &str) -> Result<f64, Box<dyn Error>> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(video_path)
        .output()?;

    let duration_str = String::from_utf8(output.stdout)?;
    let duration: f64 = duration_str.trim().parse()?;
    Ok(duration)
}

pub fn extract_frames_using_videotools(
    video_path: &str,
    num_frames: Option<u32>,
    quiet: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    // Construct the ffmpeg command
    let num_frames = num_frames.unwrap_or(5); // Default to 5 frames if not provided

    let output_dir = create_output_directory(video_path)?;
    let duration = get_video_duration(video_path)?;
    if !quiet {
        println!("duration:{:?}", duration);
    }
    let interval = duration / num_frames as f64; // Time between frames

    // Construct the ffmpeg command
    let output_pattern = output_dir.join("output-%04d.png");

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("fps=1/{}", interval))
        .arg(output_pattern.clone());

    // Optionally disable stdout and stderr output
    if quiet {
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }

    let status = cmd.status()?;

    if !status.success() {
        return Err("ffmpeg command failed".into());
    }

    // Collect the extracted frame paths
    let mut frame_paths = Vec::new();
    let frame_pattern = Path::new(&output_pattern)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let frame_prefix = &frame_pattern[..frame_pattern.len() - 8]; // Assuming output pattern is output-%04d.png

    for entry in fs::read_dir(&output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("png") {
            if path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(frame_prefix)
            {
                frame_paths.push(path.to_str().unwrap().to_string());
            }
        }
    }

    Ok(frame_paths)
}
// Function to extract frames from the video and save them as PNGs
pub fn extract_frames(video_path: &str, interval_sec: u64) -> Result<Vec<String>, Box<dyn Error>> {
    let file_path = Path::new(video_path);
    let frame_source = FileSource::new(file_path, (200, 200))?;
    let mut frame_count = 0;
    let mut frame_paths = Vec::new();

    // Calculate the number of frames to skip based on the interval
    let frame_interval = interval_sec as usize;

    for (index, frame) in frame_source.into_iter().enumerate() {
        if let Ok(Some(png_img_data)) = frame {
            // Save frame only if it matches the interval
            if index % frame_interval == 0 {
                let output_filename = format!("frame_{}.png", frame_count);
                let mut output_file = File::create(&output_filename)?;
                output_file.write_all(&png_img_data)?;
                frame_paths.push(output_filename);
                println!("Saved frame: frame_{}.png", frame_count);
                frame_count += 1;
            }
        }
    }

    Ok(frame_paths) // Return the list of saved frame file paths
}
