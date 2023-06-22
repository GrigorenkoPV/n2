//! A single hash over input attributes is recorded and used to determine when
//! those inputs change.
//!
//! See "Manifests instead of mtime order" in
//!   https://neugierig.org/software/blog/2022/03/n2.html

use crate::graph::{Build, FileId, FileState, Graph, MTime, RspFile};
use std::{
    fmt::Write,
    hash::{self, Hasher},
    time::SystemTime,
};

/// Hash value used to identify a given instance of a Build's execution;
/// compared to verify whether a Build is up to date.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Hash(pub u64);

/// A trait for computing the build's hash.  Indirected as a trait so we can
/// implement it a second time for "-d explain" debug purposes.
trait BuildHasher {
    /// Write a list of files+mtimes.  desc is used only for "-d explain" output.
    fn write_files<'a>(
        &mut self,
        desc: &str,
        graph: &Graph,
        file_state: &FileState,
        ids: &[FileId],
    );
    fn write_rsp(&mut self, rspfile: &RspFile);
    fn write_cmdline(&mut self, cmdline: &str);
}

fn get_fileid_status<'a>(
    graph: &'a Graph,
    file_state: &FileState,
    id: FileId,
) -> (&'a str, SystemTime) {
    let name = &graph.file(id).name;
    let mtime = file_state
        .get(id)
        .unwrap_or_else(|| panic!("no state for {:?}", name));
    let mtime = match mtime {
        MTime::Stamp(mtime) => mtime,
        MTime::Missing => panic!("missing file: {:?}", name),
    };
    (name.as_str(), mtime)
}

/// The BuildHasher used during normal builds, designed to not serialize too much.
#[derive(Default)]
struct TerseHash(std::collections::hash_map::DefaultHasher);

const UNIT_SEPARATOR: u8 = 0x1F;

impl TerseHash {
    fn write_string(&mut self, string: &str) {
        std::hash::Hash::hash(string, &mut self.0);
    }

    fn write_separator(&mut self) {
        self.0.write_u8(UNIT_SEPARATOR);
    }

    fn finish(&mut self) -> Hash {
        Hash(self.0.finish())
    }
}

impl BuildHasher for TerseHash {
    fn write_files<'a>(
        &mut self,
        _desc: &str,
        graph: &Graph,
        file_state: &FileState,
        ids: &[FileId],
    ) {
        for &id in ids {
            let (name, mtime) = get_fileid_status(graph, file_state, id);
            self.write_string(name);
            std::hash::Hash::hash(&mtime, &mut self.0);
        }
        self.write_separator();
    }

    fn write_cmdline(&mut self, cmdline: &str) {
        self.write_string(cmdline);
        self.write_separator();
    }

    fn write_rsp(&mut self, rspfile: &RspFile) {
        hash::Hash::hash(rspfile, &mut self.0);
    }
}

fn hash_build_with_hasher<H: BuildHasher>(
    hasher: &mut H,
    graph: &Graph,
    file_state: &FileState,
    build: &Build,
) {
    hasher.write_files("in", graph, file_state, build.dirtying_ins());
    hasher.write_files("discovered", graph, file_state, build.discovered_ins());
    hasher.write_cmdline(build.cmdline.as_deref().unwrap_or(""));
    if let Some(rspfile) = &build.rspfile {
        hasher.write_rsp(rspfile);
    }
    hasher.write_files("out", graph, file_state, build.outs());
}

// Hashes the inputs of a build to compute a signature.
// Prerequisite: all referenced files have already been stat()ed and are present.
// (It doesn't make sense to hash a build with missing files, because it's out
// of date regardless of the state of the other files.)
pub fn hash_build(graph: &Graph, file_state: &FileState, build: &Build) -> Hash {
    let mut hasher = TerseHash::default();
    hash_build_with_hasher(&mut hasher, graph, file_state, build);
    hasher.finish()
}

/// A BuildHasher that records human-readable text for "-d explain" debugging.
#[derive(Default)]
struct ExplainHash {
    text: String,
}

impl BuildHasher for ExplainHash {
    fn write_files<'a>(
        &mut self,
        desc: &str,
        graph: &Graph,
        file_state: &FileState,
        ids: &[FileId],
    ) {
        write!(&mut self.text, "{desc}:\n").unwrap();
        for &id in ids {
            let (name, mtime) = get_fileid_status(graph, file_state, id);
            let millis = mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            write!(&mut self.text, "  {millis} {name}\n").unwrap();
        }
    }

    fn write_rsp(&mut self, rspfile: &RspFile) {
        write!(&mut self.text, "rspfile path: {}\n", rspfile.path.display()).unwrap();

        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write(rspfile.content.as_bytes());
        write!(&mut self.text, "rspfile hash: {:x}\n", h.finish()).unwrap();
    }

    fn write_cmdline(&mut self, cmdline: &str) {
        write!(&mut self.text, "cmdline: {}\n", cmdline).unwrap();
    }
}

/// Logs human-readable state of all the inputs used for hashing a given build.
/// Used for "-d explain" debugging output.
pub fn explain_hash_build(graph: &Graph, file_state: &FileState, build: &Build) -> String {
    let mut explainer = ExplainHash::default();
    hash_build_with_hasher(&mut explainer, graph, file_state, build);
    explainer.text
}