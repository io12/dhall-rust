// use dhall_core::{Expr, FilePrefix, Import, ImportLocation, ImportMode, X};
use dhall_core::{Expr, Import, StringLike, X};
// use std::path::Path;
use dhall_core::*;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

pub fn panic_imports<Label: StringLike, S: Clone>(
    expr: &Expr<Label, S, Import>,
) -> Expr<Label, S, X> {
    let no_import = |i: &Import| -> X { panic!("ahhh import: {:?}", i) };
    expr.map_embed(&no_import)
}

/// A root from which to resolve relative imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRoot {
    LocalDir(PathBuf),
}

fn resolve_import(
    import: &Import,
    root: &ImportRoot,
) -> Result<Expr<String, X, X>, DhallError> {
    use self::ImportRoot::*;
    use dhall_core::FilePrefix::*;
    use dhall_core::ImportLocation::*;
    let cwd = match root {
        LocalDir(cwd) => cwd,
    };
    match &import.location {
        Local(prefix, path) => {
            let path = match prefix {
                Parent => cwd.parent().unwrap().join(path),
                _ => unimplemented!("{:?}", import),
            };
            load_dhall_file(&path, true)
        }
    }
}

#[derive(Debug)]
pub enum DhallError {
    ParseError(parser::ParseError),
    IOError(std::io::Error),
}
impl From<parser::ParseError> for DhallError {
    fn from(e: parser::ParseError) -> Self {
        DhallError::ParseError(e)
    }
}
impl From<std::io::Error> for DhallError {
    fn from(e: std::io::Error) -> Self {
        DhallError::IOError(e)
    }
}
impl fmt::Display for DhallError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use self::DhallError::*;
        match self {
            ParseError(e) => e.fmt(f),
            IOError(e) => e.fmt(f),
        }
    }
}

pub fn load_dhall_file(
    f: &Path,
    resolve_imports: bool,
) -> Result<Expr<String, X, X>, DhallError> {
    let mut buffer = String::new();
    File::open(f)?.read_to_string(&mut buffer)?;
    let expr = parser::parse_expr(&*buffer)?;
    let expr = expr.take_ownership_of_labels();
    let expr = if resolve_imports {
        let root = ImportRoot::LocalDir(f.parent().unwrap().to_owned());
        let resolve = |import: &Import| -> Expr<String, X, X> {
            resolve_import(import, &root).unwrap()
        };
        let expr = expr.map_embed(&resolve).squash_embed();
        expr
    } else {
        panic_imports(&expr)
    };
    Ok(expr)
}