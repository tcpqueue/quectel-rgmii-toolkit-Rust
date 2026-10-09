//! Embedded adb and module files. They are unpacked once per release into
//! `%LOCALAPPDATA%\SimpleAdmin\runtime\<payload id>` and reused, so a running adb server never
//! leaves undeletable copies in the temporary folder.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

static PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.bin"));
pub const ID: &str = env!("SA_PAYLOAD_ID");
const MAGIC: &[u8] = b"SAPAYLOAD1\n";
const COMPLETE: &str = ".complete";

pub struct Entry<'a> {
    pub name: &'a str,
    pub digest: &'a [u8],
    pub data: &'a [u8],
}

/// Splits the decompressed payload into entries, rejecting unsafe or truncated records.
pub fn entries(raw: &[u8]) -> Result<Vec<Entry<'_>>> {
    let mut rest = raw.strip_prefix(MAGIC).context("安装包资源格式无效")?;
    let mut take = |n: usize| -> Result<&[u8]> {
        if rest.len() < n {
            bail!("安装包资源已损坏")
        }
        let (head, tail) = rest.split_at(n);
        rest = tail;
        Ok(head)
    };
    let count = u32::from_le_bytes(take(4)?.try_into()?) as usize;
    let mut entries = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let len = u16::from_le_bytes(take(2)?.try_into()?) as usize;
        let name = std::str::from_utf8(take(len)?)?;
        let size = u64::from_le_bytes(take(8)?.try_into()?) as usize;
        let digest = take(32)?;
        let data = take(size)?;
        let path = Path::new(name);
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            bail!("安装包资源路径无效")
        }
        if Sha256::digest(data).as_slice() != digest {
            bail!("安装包资源校验失败：{name}")
        }
        entries.push(Entry { name, digest, data });
    }
    Ok(entries)
}

fn up_to_date(dir: &Path, entries: &[Entry]) -> bool {
    dir.join(COMPLETE).is_file()
        && entries.iter().all(|e| {
            fs::read(dir.join(e.name))
                .is_ok_and(|data| Sha256::digest(&data).as_slice() == e.digest)
        })
}

/// Unpacks the embedded files under `base` and returns the directory that holds them.
pub fn extract(base: &Path) -> Result<PathBuf> {
    let raw = miniz_oxide::inflate::decompress_to_vec(PACKED)
        .map_err(|e| anyhow::anyhow!("安装包资源解压失败：{e:?}"))?;
    let entries = entries(&raw)?;
    let target = base.join(ID);
    if up_to_date(&target, &entries) {
        return Ok(target);
    }
    fs::create_dir_all(base)?;
    let staging = base.join(format!(".{ID}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    for entry in &entries {
        let path = staging.join(entry.name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, entry.data)?;
    }
    fs::write(staging.join(COMPLETE), ID)?;
    // An older copy may be in use by a running adb server; replace it only when possible.
    if target.exists() && fs::remove_dir_all(&target).is_err() {
        for entry in &entries {
            let _ = fs::copy(staging.join(entry.name), target.join(entry.name));
        }
        let _ = fs::remove_dir_all(&staging);
        if up_to_date(&target, &entries) {
            return Ok(target);
        }
        bail!("运行目录被占用且无法更新：{}", target.display())
    }
    fs::rename(&staging, &target)?;
    // Remove runtimes left by earlier releases; ones still used by adb are skipped.
    if let Ok(dir) = fs::read_dir(base) {
        for old in dir.flatten() {
            if old.file_name() != ID {
                let _ = fs::remove_dir_all(old.path());
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_payload_is_complete_and_verified() {
        let raw = miniz_oxide::inflate::decompress_to_vec(PACKED).unwrap();
        let entries = entries(&raw).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name).collect();
        for required in [
            "adb.exe",
            "AdbWinApi.dll",
            "AdbWinUsbApi.dll",
            "development/SHA256SUMS",
            "development/install_simpleadmin_rust.sh",
            "development/diagnose_simpleadmin.sh",
            "development/simpleadmin/simpleadmin-httpd.armv7",
            "development/simpleadmin/www/index.html",
        ] {
            assert!(names.contains(&required), "{required} missing");
        }
        let dir = tempfile::tempdir().unwrap();
        let first = extract(dir.path()).unwrap();
        assert!(first.join("development/SHA256SUMS").is_file());
        // A damaged file is detected and rewritten on the next start.
        fs::write(first.join("development/SHA256SUMS"), "tampered").unwrap();
        let again = extract(dir.path()).unwrap();
        assert_eq!(first, again);
        assert_ne!(
            fs::read_to_string(again.join("development/SHA256SUMS")).unwrap(),
            "tampered"
        );
    }
    #[test]
    fn rejects_traversal_and_corruption() {
        let mut raw = MAGIC.to_vec();
        raw.extend(1u32.to_le_bytes());
        let name = b"../evil";
        raw.extend((name.len() as u16).to_le_bytes());
        raw.extend(name);
        raw.extend(1u64.to_le_bytes());
        raw.extend(Sha256::digest(b"x"));
        raw.push(b'x');
        assert!(entries(&raw).is_err());
        assert!(entries(&raw[..raw.len() - 1]).is_err());
    }
}
