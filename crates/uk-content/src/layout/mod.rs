pub mod bntx;

use roead::sarc::*;
use serde::{Deserialize, Serialize};
use smartstring::alias::String;

use crate::{
    prelude::*,
    util::{HashSet, IndexMap},
    Result, UKError,
};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct LayoutArchive(pub IndexMap<String, Vec<u8>>);

impl TryFrom<&'_ Sarc<'_>> for LayoutArchive {
    type Error = UKError;

    fn try_from(sarc: &'_ Sarc) -> Result<Self> {
        Ok(Self(
            sarc.files()
                .filter_map(|f| f.name.map(|n| (n.into(), f.data.to_vec())))
                .collect(),
        ))
    }
}

impl Mergeable for LayoutArchive {
    fn diff(&self, other: &Self) -> Self {
        Self(
            other
                .0
                .iter()
                .filter(|&(n, d)| self.0.get(n) != Some(d))
                .map(|(n, d)| (n.clone(), d.clone()))
                .collect(),
        )
    }

    fn merge(&self, diff: &Self) -> Self {
        let keys: HashSet<String> = self.0.keys().chain(diff.0.keys()).cloned().collect();
        Self(
            keys.into_iter()
                .map(|k| {
                    // Texture atlases are merged texture by texture, so two mods that
                    // each change different textures in the same BNTX both apply.
                    let v = match (self.0.get(&k), diff.0.get(&k)) {
                        (Some(base), Some(new)) if k.ends_with(".bntx") && base != new => {
                            bntx::merge_bntx(base, new).unwrap_or_else(|| {
                                log::warn!("Could not merge texture atlas {k}, replacing it");
                                new.to_vec()
                            })
                        }
                        (_, Some(new)) => new.to_vec(),
                        (Some(base), None) => base.to_vec(),
                        (None, None) => unreachable!("Layout archive merging failed"),
                    };
                    (k, v)
                })
                .collect(),
        )
    }
}

impl Resource for LayoutArchive {
    fn from_binary(data: impl AsRef<[u8]>) -> Result<Self> {
        let sarc = Sarc::new(data.as_ref())?;
        Self::try_from(&sarc)
    }

    fn into_binary(self, endian: Endian) -> Vec<u8> {
        SarcWriter::new(endian.into())
            .with_legacy_mode(true)
            .with_min_alignment(4)
            .with_files(self.0)
            .to_binary()
    }

    fn path_matches(path: impl AsRef<std::path::Path>) -> bool {
        path.as_ref()
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.ends_with("blarc"))
            .unwrap_or(false)
    }
}
