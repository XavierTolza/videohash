use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::fs::File;
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
/// Get the total number of frames in a video file using ffprobe.
/// This provides more accurate frame counting than duration-based calculations.
fn get_video_frame_count(video_path: &str) -> Result<u64, Box<dyn Error>> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-count_packets")
        .arg("-show_entries")
        .arg("stream=nb_read_packets")
        .arg("-of")
        .arg("csv=p=0") // Output as CSV without headers to get just the number
        .arg(video_path)
        .output()?;

    let frame_count_str = String::from_utf8(output.stdout)?;
    let frame_count: u64 = frame_count_str.trim().parse()?;
    Ok(frame_count)
}

/// Extract frames from a video using exact frame indexes for precise control.
/// This approach uses ffmpeg's select filter to extract frames by their exact
/// frame numbers rather than time-based intervals, providing more accurate results.
pub fn extract_frames_using_videotools<const NUM_FRAMES: usize>(
    video_path: &str,
    quiet: bool,
) -> Result<[String; NUM_FRAMES], Box<dyn Error>> {
    let output_dir = create_output_directory(video_path)?;
    let total_frames = get_video_frame_count(video_path)?;

    if !quiet {
        println!("Total frames: {}", total_frames);
    }

    // Calculate exact frame indexes to extract (evenly distributed)
    let mut frame_indexes = Vec::new();
    if NUM_FRAMES == 1 {
        // For single frame, extract from the middle
        frame_indexes.push(total_frames / 2);
    } else {
        // For multiple frames, distribute evenly across the video
        // This ensures we get frames from start, middle, and end portions
        for i in 0..NUM_FRAMES {
            let frame_index = (i as u64 * (total_frames - 1)) / (NUM_FRAMES as u64 - 1);
            frame_indexes.push(frame_index);
        }
    }

    if !quiet {
        println!("Extracting frames at indexes: {:?}", frame_indexes);
    }

    // Build the select filter expression using exact frame numbers
    // Format: eq(n,frame1)+eq(n,frame2)+... where + means OR
    let select_expr = frame_indexes
        .iter()
        .map(|&idx| format!("eq(n\\,{})", idx))
        .collect::<Vec<_>>()
        .join("+");

    let output_pattern = output_dir.join("frame_%04d.png");

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(format!("select='{}'", select_expr))
        .arg("-vsync")
        .arg("vfr") // Variable frame rate to preserve selected frames
        .arg("-q:v")
        .arg("2") // High quality setting
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

    for entry in fs::read_dir(&output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("png") {
            if path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("frame_")
            {
                frame_paths.push(path.to_str().unwrap().to_string());
            }
        }
    }

    // Sort paths to ensure consistent ordering
    frame_paths.sort();

    let nframes_found = frame_paths.len();
    if nframes_found != NUM_FRAMES {
        return Err(format!("Expected {} frames, but got {}", NUM_FRAMES, nframes_found).into());
    }

    Ok(frame_paths
        .try_into()
        .unwrap_or_else(|_| panic!("Expected {} frames, but got {}", NUM_FRAMES, nframes_found)))
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
#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a simple test video using ffmpeg
    fn create_test_video(path: &str, duration_sec: u32) -> Result<(), Box<dyn Error>> {
        let status = Command::new("ffmpeg")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg(format!(
                "testsrc=duration={}:size=320x240:rate=30",
                duration_sec
            ))
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-y") // Overwrite output file
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;

        if !status.success() {
            return Err("Failed to create test video".into());
        }
        Ok(())
    }

    fn cleanup_test_video(path: &str) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_create_output_directory() {
        let test_video = "test_video.mp4";
        let _ = File::create(test_video).unwrap();

        let result = create_output_directory(test_video);
        assert!(result.is_ok());

        let output_dir = result.unwrap();
        assert!(output_dir.exists());
        assert!(output_dir.to_str().unwrap().contains("videohash_"));

        // Cleanup
        let _ = fs::remove_dir_all(output_dir);
        cleanup_test_video(test_video);
    }

    #[test]
    fn test_get_video_frame_count() {
        let test_video = "test_frame_count.mp4";

        // Create a 2-second test video at 30fps (should have ~60 frames)
        if create_test_video(test_video, 2).is_ok() {
            let result = get_video_frame_count(test_video);
            assert!(result.is_ok());

            let frame_count = result.unwrap();
            // Allow some tolerance in frame count due to encoding
            assert!(frame_count >= 50 && frame_count <= 70);

            cleanup_test_video(test_video);
        }
    }

    #[test]
    fn test_extract_frames_using_videotools_single_frame() {
        let test_video = "test_single_frame.mp4";

        if create_test_video(test_video, 3).is_ok() {
            let result = extract_frames_using_videotools::<1>(test_video, true);
            assert!(result.is_ok());

            let frame_paths = result.unwrap();
            assert_eq!(frame_paths.len(), 1);
            assert!(Path::new(&frame_paths[0]).exists());

            // Cleanup frames
            for path in &frame_paths {
                let _ = fs::remove_file(path);
            }

            // Cleanup output directory
            if let Ok(output_dir) = create_output_directory(test_video) {
                let _ = fs::remove_dir_all(output_dir);
            }

            cleanup_test_video(test_video);
        }
    }

    #[test]
    fn test_extract_frames_using_videotools_multiple_frames() {
        let test_video = "test_multiple_frames.mp4";

        if create_test_video(test_video, 5).is_ok() {
            let result = extract_frames_using_videotools::<5>(test_video, true);
            assert!(result.is_ok());

            let frame_paths = result.unwrap();
            assert_eq!(frame_paths.len(), 5);

            // Verify all frames exist
            for path in &frame_paths {
                assert!(Path::new(path).exists());
                assert!(path.contains("frame_"));
                assert!(path.ends_with(".png"));
            }

            // Cleanup frames
            for path in &frame_paths {
                let _ = fs::remove_file(path);
            }

            // Cleanup output directory
            if let Ok(output_dir) = create_output_directory(test_video) {
                let _ = fs::remove_dir_all(output_dir);
            }

            cleanup_test_video(test_video);
        }
    }

    #[test]
    fn test_extract_frames_using_videotools_nonexistent_file() {
        let result = extract_frames_using_videotools::<3>("nonexistent_video.mp4", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_frames_using_videotools_frame_ordering() {
        let test_video = "test_frame_ordering.mp4";

        if create_test_video(test_video, 4).is_ok() {
            let result = extract_frames_using_videotools::<3>(test_video, true);
            assert!(result.is_ok());

            let frame_paths = result.unwrap();
            assert_eq!(frame_paths.len(), 3);

            // Verify frames are in sorted order
            for i in 1..frame_paths.len() {
                assert!(frame_paths[i - 1] < frame_paths[i]);
            }

            // Cleanup
            for path in &frame_paths {
                let _ = fs::remove_file(path);
            }

            if let Ok(output_dir) = create_output_directory(test_video) {
                let _ = fs::remove_dir_all(output_dir);
            }

            cleanup_test_video(test_video);
        }
    }

    #[test]
    fn test_extract_frames_using_videotools_quiet_mode() {
        let test_video = "test_quiet_mode.mp4";

        if create_test_video(test_video, 2).is_ok() {
            // Test both quiet=true and quiet=false
            let result_quiet = extract_frames_using_videotools::<2>(test_video, true);
            assert!(result_quiet.is_ok());

            let frame_paths = result_quiet.unwrap();
            assert_eq!(frame_paths.len(), 2);

            // Cleanup
            for path in &frame_paths {
                let _ = fs::remove_file(path);
            }

            if let Ok(output_dir) = create_output_directory(test_video) {
                let _ = fs::remove_dir_all(output_dir);
            }

            cleanup_test_video(test_video);
        }
    }

    #[test]
    fn test_get_video_frame_count_invalid_file() {
        let result = get_video_frame_count("invalid_video.mp4");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_output_directory_invalid_path() {
        let result = create_output_directory("/invalid/path/to/video.mp4");
        assert!(result.is_err());
    }
}
