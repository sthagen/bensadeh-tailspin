use crate::io::reader::StreamEvent::{Ended, Started};
use crate::io::reader::{AsyncLineReader, StreamEvent};
use miette::{Context, IntoDiagnostic, Result};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct FileReader {
    reader: BufReader<tokio::fs::File>,
    buf: Vec<u8>,
    initial_lines: Option<Vec<String>>,
    has_emitted_start_event: bool,
    terminate_after_first_read: bool,
}

impl FileReader {
    pub async fn new<P: AsRef<Path>>(file_path: P, terminate_after_first_read: bool) -> Result<FileReader> {
        let file_path = std::fs::canonicalize(file_path.as_ref())
            .into_diagnostic()
            .wrap_err("Could not canonicalize file path")?;

        let mut file = tokio::fs::File::open(&file_path)
            .await
            .into_diagnostic()
            .wrap_err("Could not open file")?;

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .await
            .into_diagnostic()
            .wrap_err("Could not read file")?;

        let content = String::from_utf8_lossy(&bytes);
        let initial_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

        // File position is now at end of initial content; wrap in BufReader for tailing
        let reader = BufReader::new(file);

        Ok(Self {
            reader,
            buf: Vec::new(),
            initial_lines: Some(initial_lines),
            has_emitted_start_event: false,
            terminate_after_first_read,
        })
    }

    async fn next_line(&mut self) -> Result<String> {
        loop {
            let bytes_read = self
                .reader
                .read_until(b'\n', &mut self.buf)
                .await
                .into_diagnostic()
                .wrap_err("Could not read next line")?;

            if bytes_read == 0 {
                // Detect file truncation: if the file shrank past our position, restart from the beginning
                let file_size = self
                    .reader
                    .get_ref()
                    .metadata()
                    .await
                    .into_diagnostic()
                    .wrap_err("Could not stat file")?
                    .len();
                let position = self
                    .reader
                    .stream_position()
                    .await
                    .into_diagnostic()
                    .wrap_err("Could not get stream position")?;

                if file_size < position {
                    self.reader
                        .seek(std::io::SeekFrom::Start(0))
                        .await
                        .into_diagnostic()
                        .wrap_err("Could not seek to start after truncation")?;
                    self.buf.clear();
                }

                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }

            if self.buf.ends_with(b"\n") {
                let line_end = if self.buf.ends_with(b"\r\n") {
                    self.buf.len() - 2
                } else {
                    self.buf.len() - 1
                };
                let line = String::from_utf8_lossy(&self.buf[..line_end]).into_owned();
                self.buf.clear();
                return Ok(line);
            }

            // Partial line at EOF — wait for more data
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

impl AsyncLineReader for FileReader {
    async fn next(&mut self) -> Result<StreamEvent> {
        if let Some(lines) = self.initial_lines.take() {
            return Ok(StreamEvent::Lines(lines));
        }

        if !self.has_emitted_start_event {
            self.has_emitted_start_event = true;
            return Ok(Started);
        }

        if self.terminate_after_first_read {
            return Ok(Ended);
        }

        let line = self.next_line().await?;
        Ok(StreamEvent::Line(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::reader::StreamEvent::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use tempfile::tempdir;
    use tokio::time::{Duration, sleep, timeout};

    #[tokio::test]
    async fn test_read_exactly_n_lines() -> Result<()> {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.log");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "line1").unwrap();
        writeln!(file, "line2").unwrap();
        writeln!(file, "line3").unwrap();

        let mut reader = FileReader::new(file_path, false).await?;

        let event = reader.next().await?;
        match event {
            Lines(lines) => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "line1");
                assert_eq!(lines[1], "line2");
                assert_eq!(lines[2], "line3");
            }
            _ => panic!("Expected StreamEvent::Lines(...)"),
        }

        let event = reader.next().await?;
        match event {
            Started => {}
            _ => panic!("Expected StreamEvent::Started"),
        }

        let result = timeout(Duration::from_millis(200), reader.next()).await;

        assert!(
            result.is_err(),
            "Entire file has been read, next() should not return anything: {:?}",
            result
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_terminate_after_first_read() -> Result<()> {
        let test_result = timeout(Duration::from_millis(1000), async {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.log");

            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "only_line").unwrap();

            let mut reader = FileReader::new(file_path, true).await?;

            let first_event = reader.next().await?;
            match first_event {
                Lines(lines) => {
                    assert_eq!(lines.len(), 1);
                    assert_eq!(lines[0], "only_line");
                }
                _ => panic!("Expected StreamEvent::Lines(...)"),
            }

            let second_event = reader.next().await?;
            match second_event {
                Started => {}
                _ => panic!("Expected StreamEvent::Started"),
            }

            let third_event = reader.next().await?;
            match third_event {
                Ended => {}
                _ => panic!("Expected StreamEvent::Ended"),
            }

            Ok(())
        })
        .await;

        test_result.unwrap_or_else(|_| Err(miette::miette!("Test timed out!")))
    }

    #[tokio::test]
    async fn test_append_new_lines_after_initial_read() -> Result<()> {
        let dir = tempdir().into_diagnostic()?;
        let file_path = dir.path().join("test_append.log");

        let mut file = File::create(&file_path).into_diagnostic()?;
        writeln!(file, "initial1").into_diagnostic()?;
        writeln!(file, "initial2").into_diagnostic()?;

        let mut reader = FileReader::new(file_path.as_path(), false).await?;
        let event = reader.next().await?;
        match event {
            Lines(lines) => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "initial1");
                assert_eq!(lines[1], "initial2");
            }
            _ => panic!("Expected StreamEvent::Lines(...)"),
        }

        let event = reader.next().await?;
        assert!(matches!(event, Started));

        let mut file = OpenOptions::new().append(true).open(&file_path).into_diagnostic()?;
        writeln!(file, "appended1").into_diagnostic()?;
        writeln!(file, "appended2").into_diagnostic()?;

        sleep(Duration::from_millis(200)).await;

        let event = timeout(Duration::from_millis(1000), reader.next())
            .await
            .into_diagnostic()
            .wrap_err("Timed out waiting for appended1")?;
        match event? {
            Line(line) => assert_eq!(line, "appended1"),
            _ => panic!("Expected StreamEvent::Line(...) with appended1"),
        }

        let event = timeout(Duration::from_millis(1000), reader.next())
            .await
            .into_diagnostic()
            .wrap_err("Timed out waiting for appended2")?;
        match event? {
            Line(line) => assert_eq!(line, "appended2"),
            _ => panic!("Expected StreamEvent::Line(...) with appended2"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_empty_file() -> Result<()> {
        let test_result = timeout(Duration::from_millis(1000), async {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("empty.log");
            File::create(&file_path).unwrap();

            let mut reader = FileReader::new(file_path, true).await?;

            let event = reader.next().await?;
            match event {
                Lines(lines) => assert!(lines.is_empty()),
                _ => panic!("Expected StreamEvent::Lines(vec![])"),
            }

            let event = reader.next().await?;
            assert!(matches!(event, Started));

            let event = reader.next().await?;
            assert!(matches!(event, Ended));

            Ok(())
        })
        .await;

        test_result.unwrap_or_else(|_| Err(miette::miette!("Test timed out!")))
    }

    #[tokio::test]
    async fn test_no_trailing_newline() -> Result<()> {
        let test_result = timeout(Duration::from_millis(1000), async {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("no_trailing.log");

            let mut file = File::create(&file_path).unwrap();
            write!(file, "line1\nline2").unwrap();

            let mut reader = FileReader::new(file_path, true).await?;

            let event = reader.next().await?;
            match event {
                Lines(lines) => {
                    assert_eq!(lines.len(), 2);
                    assert_eq!(lines[0], "line1");
                    assert_eq!(lines[1], "line2");
                }
                _ => panic!("Expected StreamEvent::Lines(...)"),
            }

            Ok(())
        })
        .await;

        test_result.unwrap_or_else(|_| Err(miette::miette!("Test timed out!")))
    }

    #[tokio::test]
    async fn test_crlf_line_endings() -> Result<()> {
        let dir = tempdir().into_diagnostic()?;
        let file_path = dir.path().join("crlf.log");

        {
            let mut file = File::create(&file_path).into_diagnostic()?;
            file.write_all(b"line1\r\nline2\r\n").into_diagnostic()?;
        }

        let mut reader = FileReader::new(file_path.as_path(), false).await?;

        let event = reader.next().await?;
        match event {
            Lines(lines) => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "line1");
                assert_eq!(lines[1], "line2");
            }
            _ => panic!("Expected StreamEvent::Lines(...)"),
        }

        let event = reader.next().await?;
        assert!(matches!(event, Started));

        // Append a CRLF line in follow mode
        {
            let mut file = OpenOptions::new().append(true).open(&file_path).into_diagnostic()?;
            file.write_all(b"appended\r\n").into_diagnostic()?;
        }

        sleep(Duration::from_millis(200)).await;

        let event = timeout(Duration::from_millis(1000), reader.next())
            .await
            .into_diagnostic()
            .wrap_err("Timed out waiting for appended CRLF line")?;
        match event? {
            Line(line) => assert_eq!(line, "appended"),
            _ => panic!("Expected StreamEvent::Line(\"appended\")"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_non_utf8_content() -> Result<()> {
        let test_result = timeout(Duration::from_millis(1000), async {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("non_utf8.log");

            {
                let mut file = File::create(&file_path).unwrap();
                // Write invalid UTF-8: 0xFF 0xFE are not valid UTF-8 byte sequences
                file.write_all(b"hello \xff\xfe world\n").unwrap();
            }

            let mut reader = FileReader::new(file_path, true).await?;

            let event = reader.next().await?;
            match event {
                Lines(lines) => {
                    assert_eq!(lines.len(), 1);
                    assert!(lines[0].contains("hello"));
                    assert!(lines[0].contains("world"));
                    assert!(lines[0].contains('\u{FFFD}'));
                }
                _ => panic!("Expected StreamEvent::Lines(...)"),
            }

            Ok(())
        })
        .await;

        test_result.unwrap_or_else(|_| Err(miette::miette!("Test timed out!")))
    }

    #[tokio::test]
    async fn test_non_utf8_in_follow_mode() -> Result<()> {
        let dir = tempdir().into_diagnostic()?;
        let file_path = dir.path().join("non_utf8_follow.log");

        {
            let mut file = File::create(&file_path).into_diagnostic()?;
            writeln!(file, "initial").into_diagnostic()?;
        }

        let mut reader = FileReader::new(file_path.as_path(), false).await?;

        let event = reader.next().await?;
        assert!(matches!(event, Lines(_)));

        let event = reader.next().await?;
        assert!(matches!(event, Started));

        // Append non-UTF-8 in follow mode
        {
            let mut file = OpenOptions::new().append(true).open(&file_path).into_diagnostic()?;
            file.write_all(b"caf\xe9\n").into_diagnostic()?;
        }

        sleep(Duration::from_millis(200)).await;

        let event = timeout(Duration::from_millis(1000), reader.next())
            .await
            .into_diagnostic()
            .wrap_err("Timed out waiting for non-UTF-8 line")?;
        match event? {
            Line(line) => {
                assert!(line.starts_with("caf"));
                assert!(line.contains('\u{FFFD}'));
            }
            _ => panic!("Expected StreamEvent::Line(...)"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_truncation_detection() -> Result<()> {
        let dir = tempdir().into_diagnostic()?;
        let file_path = dir.path().join("truncate.log");

        let mut file = File::create(&file_path).into_diagnostic()?;
        writeln!(file, "original1").into_diagnostic()?;
        writeln!(file, "original2").into_diagnostic()?;

        let mut reader = FileReader::new(file_path.as_path(), false).await?;

        let event = reader.next().await?;
        match event {
            Lines(lines) => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "original1");
                assert_eq!(lines[1], "original2");
            }
            _ => panic!("Expected StreamEvent::Lines(...)"),
        }

        let event = reader.next().await?;
        assert!(matches!(event, Started));

        // Truncate the file and write new, shorter content
        let mut file = File::create(&file_path).into_diagnostic()?;
        writeln!(file, "new").into_diagnostic()?;

        sleep(Duration::from_millis(200)).await;

        let event = timeout(Duration::from_millis(1000), reader.next())
            .await
            .into_diagnostic()
            .wrap_err("Timed out waiting for line after truncation")?;
        match event? {
            Line(line) => assert_eq!(line, "new"),
            _ => panic!("Expected StreamEvent::Line(\"new\")"),
        }

        Ok(())
    }
}
