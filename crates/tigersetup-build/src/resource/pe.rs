//! The Win32 resource API over a PE file on disk: what resources it has, a
//! resource's bytes, and a rewrite of the resource section through
//! `BeginUpdateResourceW` / `UpdateResourceW` / `EndUpdateResourceW`.
//!
//! Reading goes through a module handle loaded as a data file and image
//! resource — no code of the file runs — and that handle is closed before
//! an update begins, because the update rewrites the file in place.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, EnumResourceLanguagesW, EnumResourceNamesW,
    FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW,
    LoadResource, LockResource, SizeofResource, UpdateResourceW,
};
use windows_sys::core::PCWSTR;

use crate::{BuildError, Result};

/// The predefined resource types this module handles, as `winuser.h`
/// numbers them.
pub const RT_ICON: u16 = 3;
pub const RT_GROUP_ICON: u16 = 14;
pub const RT_VERSION: u16 = 16;

/// `ERROR_RESOURCE_DATA_NOT_FOUND`, `ERROR_RESOURCE_TYPE_NOT_FOUND` and
/// `ERROR_RESOURCE_NAME_NOT_FOUND`: an enumeration that found nothing.
const NOTHING_FOUND: [i32; 3] = [1812, 1813, 1814];

/// A resource name as the API carries it: a 16-bit id, or a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    Id(u16),
    Text(Vec<u16>),
}

impl Name {
    /// The `MAKEINTRESOURCE` form or the string pointer; the string stays
    /// owned by `self`.
    fn as_pcwstr(&self) -> PCWSTR {
        match self {
            Name::Id(id) => *id as usize as PCWSTR,
            Name::Text(text) => text.as_ptr(),
        }
    }

    /// Copies the name a callback was handed, which is valid only during the
    /// callback.
    unsafe fn from_callback(name: PCWSTR) -> Name {
        if (name as usize) >> 16 == 0 {
            Name::Id(name as usize as u16)
        } else {
            let mut text = Vec::new();
            let mut at = 0;
            loop {
                let unit = unsafe { *name.add(at) };
                text.push(unit);
                if unit == 0 {
                    break;
                }
                at += 1;
            }
            Name::Text(text)
        }
    }
}

/// One resource the file has, enough to delete it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existing {
    pub kind: u16,
    pub name: Name,
    pub language: u16,
}

/// One resource to write.
#[derive(Debug, Clone)]
pub struct Update {
    pub kind: u16,
    pub id: u16,
    pub data: Vec<u8>,
}

fn wide(path: &Path) -> Vec<u16> {
    OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn os_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn kind_pcwstr(kind: u16) -> PCWSTR {
    kind as usize as PCWSTR
}

/// A PE file opened for reading its resources.
pub struct Module {
    handle: HMODULE,
}

impl Module {
    pub fn open(path: &Path) -> Result<Module> {
        let file = wide(path);
        let handle = unsafe {
            LoadLibraryExW(
                file.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            )
        };
        if handle.is_null() {
            return Err(BuildError::new(
                "engine_invalid",
                format!(
                    "{} is not an executable whose resources can be read: {}",
                    path.display(),
                    os_error()
                ),
            ));
        }
        Ok(Module { handle })
    }

    /// The bytes of resource `id` of `kind` in any language, or `None` when
    /// the file has no such resource.
    pub fn resource(&self, kind: u16, id: u16) -> Result<Option<Vec<u8>>> {
        let info = unsafe { FindResourceW(self.handle, id as usize as PCWSTR, kind_pcwstr(kind)) };
        if info.is_null() {
            return Ok(None);
        }
        let size = unsafe { SizeofResource(self.handle, info) };
        let loaded = unsafe { LoadResource(self.handle, info) };
        let pointer = unsafe { LockResource(loaded) };
        if pointer.is_null() {
            return Err(BuildError::new(
                "resource_unreadable",
                format!("resource {kind}/{id} cannot be loaded: {}", os_error()),
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size as usize) };
        Ok(Some(bytes.to_vec()))
    }

    /// Every resource of the given kinds, in every language.
    pub fn existing(&self, kinds: &[u16]) -> Result<Vec<Existing>> {
        let mut out = Vec::new();
        for &kind in kinds {
            for name in self.names(kind)? {
                for language in self.languages(kind, &name)? {
                    out.push(Existing {
                        kind,
                        name: name.clone(),
                        language,
                    });
                }
            }
        }
        Ok(out)
    }

    fn names(&self, kind: u16) -> Result<Vec<Name>> {
        unsafe extern "system" fn collect(
            _module: HMODULE,
            _kind: PCWSTR,
            name: PCWSTR,
            names: isize,
        ) -> i32 {
            let names = unsafe { &mut *(names as *mut Vec<Name>) };
            names.push(unsafe { Name::from_callback(name) });
            1
        }
        let mut names: Vec<Name> = Vec::new();
        let ok = unsafe {
            EnumResourceNamesW(
                self.handle,
                kind_pcwstr(kind),
                Some(collect),
                &mut names as *mut Vec<Name> as isize,
            )
        };
        if ok == 0 && names.is_empty() {
            let error = os_error();
            if !error
                .raw_os_error()
                .is_some_and(|code| NOTHING_FOUND.contains(&code))
            {
                return Err(BuildError::new(
                    "resource_unreadable",
                    format!("resources of type {kind} cannot be enumerated: {error}"),
                ));
            }
        }
        Ok(names)
    }

    fn languages(&self, kind: u16, name: &Name) -> Result<Vec<u16>> {
        unsafe extern "system" fn collect(
            _module: HMODULE,
            _kind: PCWSTR,
            _name: PCWSTR,
            language: u16,
            languages: isize,
        ) -> i32 {
            let languages = unsafe { &mut *(languages as *mut Vec<u16>) };
            languages.push(language);
            1
        }
        let mut languages: Vec<u16> = Vec::new();
        let ok = unsafe {
            EnumResourceLanguagesW(
                self.handle,
                kind_pcwstr(kind),
                name.as_pcwstr(),
                Some(collect),
                &mut languages as *mut Vec<u16> as isize,
            )
        };
        if ok == 0 && languages.is_empty() {
            let error = os_error();
            if !error
                .raw_os_error()
                .is_some_and(|code| NOTHING_FOUND.contains(&code))
            {
                return Err(BuildError::new(
                    "resource_unreadable",
                    format!("languages of resource {kind}/{name:?} cannot be enumerated: {error}"),
                ));
            }
        }
        Ok(languages)
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.handle);
        }
    }
}

/// Rewrites the resource section of the file at `path`: `delete` first,
/// then `write` in `language`. Nothing is written when any step fails.
pub fn update(path: &Path, delete: &[Existing], write: &[Update], language: u16) -> Result<()> {
    let failed = |what: String| {
        BuildError::new(
            "resource_update_failed",
            format!("{}: {what}: {}", path.display(), os_error()),
        )
    };
    let file = wide(path);
    let handle: HANDLE = unsafe { BeginUpdateResourceW(file.as_ptr(), 0) };
    if handle.is_null() {
        return Err(failed("the resource update cannot begin".into()));
    }
    let result = (|| {
        for existing in delete {
            let ok = unsafe {
                UpdateResourceW(
                    handle,
                    kind_pcwstr(existing.kind),
                    existing.name.as_pcwstr(),
                    existing.language,
                    std::ptr::null(),
                    0,
                )
            };
            if ok == 0 {
                return Err(failed(format!(
                    "resource {}/{:?} (language {:#06x}) cannot be deleted",
                    existing.kind, existing.name, existing.language
                )));
            }
        }
        for update in write {
            let ok = unsafe {
                UpdateResourceW(
                    handle,
                    kind_pcwstr(update.kind),
                    update.id as usize as PCWSTR,
                    language,
                    update.data.as_ptr().cast(),
                    update.data.len() as u32,
                )
            };
            if ok == 0 {
                return Err(failed(format!(
                    "resource {}/{} cannot be written",
                    update.kind, update.id
                )));
            }
        }
        Ok(())
    })();
    if let Err(err) = result {
        unsafe {
            EndUpdateResourceW(handle, 1);
        }
        return Err(err);
    }
    if unsafe { EndUpdateResourceW(handle, 0) } == 0 {
        return Err(failed("the resource update cannot be committed".into()));
    }
    Ok(())
}
