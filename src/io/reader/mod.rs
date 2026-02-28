mod buffer_line_counter;
pub mod command;
pub mod file_reader;
pub mod stdin;

use crate::io::controller::Reader;
use async_trait::async_trait;
use miette::Result;

#[derive(Debug)]
pub enum StreamEvent {
    Started,
    Ended,
    Line(String),
    Lines(Vec<String>),
}

#[async_trait]
pub trait AsyncLineReader {
    async fn next(&mut self) -> Result<StreamEvent>;
}

#[async_trait]
impl AsyncLineReader for Reader {
    async fn next(&mut self) -> Result<StreamEvent> {
        match self {
            Reader::File(r) => r.next().await,
            Reader::Stdin(r) => r.next().await,
            Reader::Command(r) => r.next().await,
        }
    }
}
