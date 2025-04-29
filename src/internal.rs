use std::{io::SeekFrom, sync::Once};

use anyhow::Result;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, BufReader},
};

static INIT: Once = Once::new();

#[allow(dead_code)] // this function is used in tests
pub(crate) fn init_logger() {
    INIT.call_once(|| {
        #[cfg(test)]
        env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .init();
    });
}

#[allow(dead_code)] // this function is used in tests
pub(crate) async fn compare_files(file1: &mut File, file2: &mut File) -> Result<bool> {
    const BUFFER_SIZE: usize = 8192;

    // seek to the beginning
    file1.seek(SeekFrom::Start(0)).await?;
    file2.seek(SeekFrom::Start(0)).await?;

    // make reader
    let mut reader1 = BufReader::new(file1);
    let mut reader2 = BufReader::new(file2);

    // make buffer
    let mut buffer1 = vec![0; BUFFER_SIZE];
    let mut buffer2 = vec![0; BUFFER_SIZE];

    // compare by block
    loop {
        let bytes_read1 = reader1.read(&mut buffer1).await?;
        let bytes_read2 = reader2.read(&mut buffer2).await?;

        if bytes_read1 != bytes_read2 {
            return Ok(false);
        }

        if bytes_read1 == 0 {
            break; // EOF
        }

        if buffer1[..bytes_read1] != buffer2[..bytes_read1] {
            return Ok(false);
        }
    }

    Ok(true)
}
