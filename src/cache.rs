use std::{fs::File, path::Path};

use memmap2::Mmap;

const MAGIC: &[u8; 9] = b"GITIGNORE";
const CURRENT_VERSION: u32 = 1;

#[derive(rkyv::Archive, rkyv::Serialize)]
pub struct FileCached {
    magic: [u8; 9],
    version: u32,
    names: Vec<String>,
}

pub struct Templates {
    mmap: Mmap,
}

pub struct CacheIter<'a> {
    inner: std::slice::Iter<'a, rkyv::string::ArchivedString>,
}

impl<'a> Iterator for CacheIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(rkyv::string::ArchivedString::as_str)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for CacheIter<'_> {}

// Implements `for name in &cache`
impl<'a> IntoIterator for &'a Templates {
    type Item = &'a str;
    type IntoIter = CacheIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        // SAFETY: The buffer was verified during `load()`, and `mmap` is immutable.
        let cache = unsafe { rkyv::access_unchecked::<ArchivedFileCached>(&self.mmap) };

        CacheIter {
            inner: cache.names.as_slice().iter(),
        }
    }
}

impl FileCached {
    pub fn save<P>(path: P, names: Vec<String>) -> Result<(), Box<dyn std::error::Error>>
    where
        P: AsRef<Path>,
    {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let payload = FileCached {
            magic: *MAGIC,
            version: CURRENT_VERSION,
            names,
        };

        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)?;
        std::fs::write(path, bytes)?;

        Ok(())
    }

    pub fn load<P>(path: P) -> Result<Templates, Box<dyn std::error::Error>>
    where
        P: AsRef<Path>,
    {
        let file = File::open(path)?;

        if file.metadata()?.len() == 0 {
            return Err("cache file is empty".into());
        }

        let mmap = unsafe { memmap2::Mmap::map(&file) }?;

        let cache = rkyv::access::<ArchivedFileCached, rkyv::rancor::Error>(&mmap)?;

        if !cache.magic.eq(MAGIC) {
            return Err("cache file has different MAGIC".into());
        }

        if cache.version != CURRENT_VERSION {
            return Err(
                "cache file is not the same as CURRENT_VERSION, it might incompatible".into(),
            );
        }

        // Ok(cache.names)
        Ok(Templates { mmap })
    }
}
