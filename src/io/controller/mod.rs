use async_trait::async_trait;
use miette::Result;
use tokio::io;

use crate::config::{Config, Input, Output};
use crate::io::presenter::custom_pager::CustomPager;
use crate::io::presenter::empty::NoPresenter;
use crate::io::presenter::less::Less;
use crate::io::presenter::Present;
use crate::io::reader::command::CommandReader;
use crate::io::reader::linemux::Linemux;
use crate::io::reader::stdin::StdinReader;
use crate::io::reader::AsyncLineReader;
use crate::io::writer::stdout::StdoutWriter;
use crate::io::writer::temp_file::TempFile;
use crate::io::writer::AsyncLineWriter;
use tokio::sync::oneshot::Sender;

pub enum Reader {
    Linemux(Linemux),
    Stdin(StdinReader),
    Command(CommandReader),
}

pub enum Writer {
    TempFile(TempFile),
    Stdout(StdoutWriter),
}

pub enum PresenterImpl {
    Less(Less),
    CustomPager(CustomPager),
    NoPresenter(NoPresenter),
}

pub struct Io {
    reader: Reader,
    writer: Writer,
}

pub struct Presenter {
    presenter: PresenterImpl,
}

pub async fn get_io_and_presenter(config: Config, reached_eof_tx: Option<Sender<()>>) -> (Io, Presenter) {
    let reader = get_reader(config.input, config.start_at_end, reached_eof_tx).await;
    let (writer, presenter) = get_writer_and_presenter(config.output, config.follow).await;

    (Io { reader, writer }, Presenter { presenter })
}

async fn get_reader(input: Input, start_at_end: bool, reached_eof_tx: Option<Sender<()>>) -> Reader {
    match input {
        Input::File(file_info) => {
            Linemux::get_reader(file_info.path, file_info.line_count, start_at_end, reached_eof_tx).await
        }
        Input::Stdin => StdinReader::get_reader(reached_eof_tx),
        Input::Command(cmd) => CommandReader::get_reader(cmd, reached_eof_tx).await,
    }
}

async fn get_writer_and_presenter(output: Output, follow: bool) -> (Writer, PresenterImpl) {
    match output {
        Output::Less => {
            let result = TempFile::get_writer_result().await;
            let presenter = Less::get_presenter(result.temp_file_path, follow);

            (result.writer, presenter)
        }
        Output::CustomPager(cmd) => {
            let result = TempFile::get_writer_result().await;
            let presenter = CustomPager::get_presenter(result.temp_file_path, cmd);

            (result.writer, presenter)
        }
        Output::Stdout => {
            let writer = StdoutWriter::init();
            let presenter = NoPresenter::get_presenter();

            (writer, presenter)
        }
    }
}

#[async_trait]
impl AsyncLineReader for Io {
    async fn next_line_batch(&mut self) -> io::Result<Option<Vec<String>>> {
        self.reader.next_line_batch().await
    }
}

#[async_trait]
impl AsyncLineWriter for Io {
    async fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_line(line).await
    }
}

impl Present for Presenter {
    fn present(&self) -> Result<()> {
        self.presenter.present()
    }
}
