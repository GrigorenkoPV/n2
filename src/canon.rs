//! Path canonicalization.

use std::hint::assert_unchecked;
use std::mem::MaybeUninit;

/// An on-stack stack of values.
/// Used for tracking locations of parent components within a path.
struct StackStack<T, const CAPACITY: usize> {
    n: usize,
    vals: [MaybeUninit<T>; CAPACITY],
}

impl<T: Copy, const CAPACITY: usize> StackStack<T, CAPACITY> {
    fn new() -> Self {
        StackStack {
            n: 0,
            vals: [MaybeUninit::uninit(); CAPACITY],
        }
    }

    fn push(&mut self, val: T) {
        if self.n >= self.vals.len() {
            panic!("too many path components");
        }
        self.vals[self.n].write(val);
        self.n += 1;
    }

    fn pop(&mut self) -> Option<T> {
        if self.n > 0 {
            self.n -= 1;
            // Safety: we only access vals[i] after setting it.
            Some(unsafe { self.vals[self.n].assume_init() })
        } else {
            None
        }
    }
}

/// Lexically canonicalize a path, removing redundant components.
/// Does not access the disk, but only simplifies things like
/// "foo/./bar" => "foo/bar".
/// These paths can show up due to variable expansion in particular.
pub fn canon_path_fast(path: &mut String) {
    assert!(!path.is_empty());
    let mut components = StackStack::<usize, 60>::new();

    // Safety: we will modify the string by removing some ASCII characters in place
    // and shifting other contents left to fill the gaps,
    // so if it was valid UTF-8, it will remain that way.
    let data = unsafe { path.as_mut_vec() };
    let mut dst = 0;
    let mut src = 0;

    if let Some(b'/' | b'\\') = data.get(src) {
        src += 1;
        dst += 1;
    };

    // One iteration per path component.
    while let Some(&current) = data.get(src) {
        // Peek ahead for special path components: "/", ".", and "..".
        match current {
            b'/' | b'\\' => {
                src += 1;
                continue;
            }
            b'.' => {
                let Some(&next) = data.get(src + 1) else {
                    break; // Trailing '.', trim.
                };
                match next {
                    b'/' | b'\\' => {
                        // "./", skip.
                        src += 2;
                        continue;
                    }
                    // ".."
                    b'.' => match data.get(src + 2) {
                        None | Some(b'/' | b'\\') => {
                            // ".." component, try to back up.
                            if let Some(ofs) = components.pop() {
                                dst = ofs;
                            } else {
                                unsafe { assert_unchecked(dst <= src) };
                                data[dst] = b'.';
                                dst += 1;
                                data[dst] = b'.';
                                dst += 1;
                                if let Some(sep) = data.get(src + 2) {
                                    data[dst] = *sep;
                                    dst += 1;
                                }
                            }
                            src += 3;
                            continue;
                        }
                        _ => {
                            // Component that happens to start with "..".
                            // Handle as an ordinary component.
                        }
                    },
                    _ => {}
                }
            }
            _ => {}
        }

        // Mark this point as a possible target to pop to.
        components.push(dst);

        // Copy one path component, including trailing '/'.
        let stop = match data[src..].iter().position(|c| matches!(c, b'/' | b'\\')) {
            Some(pos) => src + pos + 1,
            None => data.len(),
        };
        unsafe { assert_unchecked(dst <= src && src <= stop && stop <= data.len()) };
        data.copy_within(src..stop, dst);
        dst += stop - src;
        src = stop;
    }

    if dst == 0 {
        data[0] = b'.';
        dst = 1;
    }
    // Safety: dst <= src <= len
    unsafe { data.set_len(dst) };
}

pub fn canon_path<T: Into<String>>(path: T) -> String {
    let mut path = path.into();
    canon_path_fast(&mut path);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    // Assert that canon path equals expected path with different path separators
    #[track_caller]
    fn assert_canon_path_eq(left: &str, right: &str) {
        assert_eq!(canon_path(left), right);
        assert_eq!(
            canon_path(left.replace('/', "\\")),
            right.replace('/', "\\")
        );
    }

    #[test]
    fn noop() {
        assert_canon_path_eq("foo", "foo");

        assert_canon_path_eq("foo/bar", "foo/bar");
    }

    #[test]
    fn dot() {
        assert_canon_path_eq("./foo", "foo");
        assert_canon_path_eq("foo/.", "foo/");
        assert_canon_path_eq("foo/./bar", "foo/bar");
        assert_canon_path_eq("./", ".");
        assert_canon_path_eq("./.", ".");
        assert_canon_path_eq("././", ".");
        assert_canon_path_eq("././.", ".");
        assert_canon_path_eq(".", ".");
    }

    #[test]
    fn not_dot() {
        assert_canon_path_eq("t/.hidden", "t/.hidden");
        assert_canon_path_eq("t/.._lib.c.o", "t/.._lib.c.o");
    }

    #[test]
    fn slash() {
        assert_canon_path_eq("/foo", "/foo");
        assert_canon_path_eq("foo//bar", "foo/bar");
    }

    #[test]
    fn parent() {
        assert_canon_path_eq("foo/../bar", "bar");

        assert_canon_path_eq("/foo/../bar", "/bar");
        assert_canon_path_eq("../foo", "../foo");
        assert_canon_path_eq("../foo/../bar", "../bar");
        assert_canon_path_eq("../../bar", "../../bar");
        assert_canon_path_eq("./../foo", "../foo");
        assert_canon_path_eq("foo/..", ".");
        assert_canon_path_eq("foo/../", ".");
        assert_canon_path_eq("foo/../../", "../");
        assert_canon_path_eq("foo/../../bar", "../bar");
    }
}
