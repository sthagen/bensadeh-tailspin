use crate::io::writer::AsyncLineWriter;
use miette::Result;

pub struct StdoutWriter {
    _private: (),
}

impl StdoutWriter {
    pub const fn new() -> StdoutWriter {
        StdoutWriter { _private: () }
    }
}

impl AsyncLineWriter for StdoutWriter {
    async fn write(&mut self, line: &str) -> Result<()> {
        println!("{}", line);

        Ok(())
    }
}
