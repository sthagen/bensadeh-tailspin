use crate::io::writer::AsyncLineWriter;
use miette::{Context, IntoDiagnostic, Result};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

pub struct TempFile {
    writer: BufWriter<File>,
}

impl TempFile {
    pub async fn new(writer: BufWriter<File>) -> Self {
        TempFile { writer }
    }
}

impl AsyncLineWriter for TempFile {
    async fn write(&mut self, line: &str) -> Result<()> {
        self.writer
            .write_all(line.as_bytes())
            .await
            .into_diagnostic()
            .wrap_err("Failed to write line to file")?;

        self.writer
            .write_all(b"\n")
            .await
            .into_diagnostic()
            .wrap_err("Failed to write line to file")?;

        // Flush after each write so the pager (e.g. less +F) sees lines immediately
        self.writer
            .flush()
            .await
            .into_diagnostic()
            .wrap_err("Error flushing temp file")?;

        Ok(())
    }
}
