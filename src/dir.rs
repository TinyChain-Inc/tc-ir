use std::{collections::BTreeMap, fmt, str::FromStr};

use pathlink::{Path, PathSegment};
use tc_error::{TCError, TCResult};

use crate::Route;

/// Directory-style router inspired by TinyChain's transactional `Dir`.
#[derive(Default)]
pub struct Dir<H> {
    entries: BTreeMap<PathSegment, DirEntry<H>>,
}

enum DirEntry<H> {
    Dir(Box<Dir<H>>),
    Handler(H),
}

impl<H: Clone> Clone for Dir<H> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<H: Clone> Clone for DirEntry<H> {
    fn clone(&self) -> Self {
        match self {
            Self::Dir(dir) => Self::Dir(Box::new((**dir).clone())),
            Self::Handler(handler) => Self::Handler(handler.clone()),
        }
    }
}

impl<H: fmt::Debug> fmt::Debug for Dir<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.entries.iter()).finish()
    }
}

impl<H: fmt::Debug> fmt::Debug for DirEntry<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dir(_) => f.write_str("Dir(...)"),
            Self::Handler(handler) => f.debug_tuple("Handler").field(handler).finish(),
        }
    }
}

impl<H> Dir<H> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Build a directory from a collection of `(path, handler)` entries.
    pub fn from_routes<I>(routes: I) -> TCResult<Self>
    where
        I: IntoIterator<Item = (Vec<PathSegment>, H)>,
    {
        let mut dir = Self::new();
        for (path, handler) in routes {
            if path.is_empty() {
                return Err(TCError::bad_request("cannot mount handler at root"));
            }
            dir.insert_segments(&path, handler)?;
        }
        Ok(dir)
    }

    fn insert_segments(&mut self, path: &[PathSegment], handler: H) -> TCResult<()> {
        let (head, tail) = path
            .split_first()
            .expect("caller ensures path is non-empty");

        use std::collections::btree_map::Entry;

        if tail.is_empty() {
            match self.entries.entry(head.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(DirEntry::Handler(handler));
                    Ok(())
                }
                Entry::Occupied(_) => Err(TCError::bad_request(format!(
                    "handler already mounted at path {}",
                    format_path(path)
                ))),
            }
        } else {
            let entry = self.entries.entry(head.clone()).or_insert_with(|| {
                DirEntry::Dir(Box::new(Dir {
                    entries: BTreeMap::new(),
                }))
            });

            match entry {
                DirEntry::Dir(dir) => dir.insert_segments(tail, handler),
                DirEntry::Handler(_) => Err(TCError::bad_request(format!(
                    "cannot mount handler below a leaf handler at {}",
                    format_path(path)
                ))),
            }
        }
    }

    fn route_path<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a H> {
        let (head, tail) = path.split_first()?;
        match self.entries.get(head) {
            Some(DirEntry::Handler(handler)) if tail.is_empty() => Some(handler),
            Some(DirEntry::Dir(dir)) => dir.route_path(tail),
            _ => None,
        }
    }
}

impl<H> Route for Dir<H> {
    type Handler = H;

    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a Self::Handler> {
        self.route_path(path)
    }
}

fn format_path(path: &[PathSegment]) -> String {
    Path::from(path).to_string()
}

/// Parse a `/foo/bar`-style path into [`PathSegment`]s for use with a [`Dir`].
pub fn parse_route_path(path: &str) -> TCResult<Vec<PathSegment>> {
    if path.is_empty() {
        return Err(TCError::bad_request("route paths must not be empty"));
    }

    let trimmed = path.trim();
    let trimmed = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Err(TCError::bad_request(
            "route paths must contain at least one segment",
        ));
    }

    trimmed
        .split('/')
        .map(|segment| {
            PathSegment::from_str(segment).map_err(|cause| {
                TCError::bad_request(format!("invalid route segment '{segment}': {cause}"))
            })
        })
        .collect()
}

/// Build a [`Dir`] from string routes with minimal boilerplate.
#[macro_export]
macro_rules! tc_library_routes {
    ($($path:expr => $handler:expr),+ $(,)?) => {{
        (|| -> tc_error::TCResult<_> {
            let routes = vec![
                $(
                    ($crate::parse_route_path($path)?, $handler)
                ),+
            ];
            $crate::Dir::from_routes(routes)
        })()
    }};
}

