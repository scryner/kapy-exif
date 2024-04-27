use std::{collections::HashMap, io::ErrorKind};

use anyhow::Result;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

use crate::ManipulateRawExif;

pub struct Heic<R> {
    r: R,
}

impl<R> Heic<R>
where
    R: AsyncRead + AsyncSeek + Send + Sync,
{
    pub fn new(r: R) -> Self {
        Self { r }
    }
}

#[async_trait]
impl<R> ManipulateRawExif for Heic<R>
where
    R: AsyncRead + AsyncSeek + Send + Sync,
{
    async fn extract(&self) -> Result<Vec<u8>> {
        unimplemented!()
    }

    async fn replace(&mut self, data: &[u8]) -> Result<()> {
        unimplemented!()
    }
}

struct DataPtr {
    offset: u64,
    length: usize,
}

struct RawBox {
    box_type: String,
    data_ptr: DataPtr,
}

impl RawBox {
    async fn from_reader<R>(r: &mut R) -> Result<Option<Self>>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        // get current offset
        let offset = r.seek(std::io::SeekFrom::Current(0)).await?;

        // read first 4 bytes from reader
        let length = {
            let mut buf = [0u8; 4];
            match r.read_exact(&mut buf).await {
                Ok(_) => (),
                Err(e) => {
                    // return None if EOF
                    if e.kind() == ErrorKind::UnexpectedEof {
                        return Ok(None);
                    } else {
                        return Err(e.into());
                    }
                }
            }

            // convert to u32 in big endian
            u32::from_be_bytes(buf)
        };

        // read next 4 bytes from reader to get box type
        let box_type = {
            let mut buf = [0u8; 4];
            r.read_exact(&mut buf).await?;

            // convert to string
            String::from_utf8(buf.to_vec())?
        };

        Ok(Some(Self {
            box_type,
            data_ptr: DataPtr {
                offset,
                length: length as usize,
            },
        }))
    }
}

struct FullBox {
    ftyp: FileTypeBox,    // ftyp
    meta: MetaBox,        // meta
    free: Option<RawBox>, // free
    media: RawBox,        // mdat
}

impl FullBox {
    async fn from_reader<R>(r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let mut ftyp: Option<FileTypeBox> = None;
        let mut meta: Option<MetaBox> = None;
        let mut free: Option<RawBox> = None;
        let mut media: Option<RawBox> = None;

        loop {
            // read raw box
            let raw_box = match RawBox::from_reader(r).await? {
                Some(b) => b,
                None => break,
            };

            let box_type = raw_box.box_type.as_str();

            match box_type {
                "ftyp" => {
                    let file_type_box = FileTypeBox::from_raw_box(&raw_box, r).await?;
                    ftyp = Some(file_type_box);
                }
                "meta" => {
                    let meta_box = MetaBox::from_raw_box(&raw_box, r).await?;
                    meta = Some(meta_box);
                }
                "free" => {
                    free = Some(raw_box);
                }
                "mdat" => {
                    media = Some(raw_box);
                }
                _ => return Err(anyhow::anyhow!("Unknown box type: {}", box_type)),
            }
        }

        if ftyp.is_none() || meta.is_none() || media.is_none() {
            return Err(anyhow::anyhow!("Missing required boxes"));
        }

        Ok(Self {
            ftyp: ftyp.unwrap(),
            meta: meta.unwrap(),
            free,
            media: media.unwrap(),
        })
    }
}

struct FileTypeBox {
    major_brand: u32,
    minor_version: u32,
    compatible_brands: Vec<u32>,
    data_ptr: DataPtr,
}

impl FileTypeBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        todo!()
    }
}

struct MetaBox {
    boxes: HashMap<String, RawBox>,
    info_items: HashMap<String, InfoItemBox>,
    data_ptr: DataPtr,
}

impl MetaBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        todo!()
    }
}

struct InfoItemBox {
    item_id: u32,
    item_type: String,
    version: u32,
    data_ptr: DataPtr,
}

// struct MetaBox<'a> {
//     handler: HandlerBox<'a>,            // hdlr
//     primary_item: RawBox<'a>,           // pitm: ignore details
//     data_information: RawBox<'a>,       // dinf: ignore details
//     item_location: ItemLocationBox<'a>, // iloc
//     item_protection: RawBox<'a>,        // iprp: ignore details
//     item_info: ItemInfoBox<'a>,         // iinf
//     ipmp_control: RawBox<'a>,           // ipmc: ignore details
//     item_reference: RawBox<'a>,         // iref: ignore details
//     item_data: RawBox<'a>,              // idat: ignore details
// }

// struct HandlerBox<'a> {
//     pre_defined: u32,
//     handler_type: u32,
//     reserved: [u32; 3],
//     name: String,
//     data_ptr: &'a [u8],
// }

// struct ItemLocationBox<'a> {
//     // iloc
//     data_ptr: &'a [u8],
// }

// struct ItemInfoBox<'a> {
//     // iinf
//     entry_count: u32,

//     data_ptr: &'a [u8],
// }

// struct  ItemInfoEntry<'a> {

//     data_ptr: &'a [u8],
// }
