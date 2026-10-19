// SPDX-License-Identifier: MIT

use std::ffi::{c_char, c_int, c_void, CStr};
use std::fmt;
use std::ptr::NonNull;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

const RTLD_LAZY: c_int = 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DynamicLibraryError {
    message: String,
}

impl DynamicLibraryError {
    fn open(candidates: &[&CStr]) -> Self {
        let names = candidates
            .iter()
            .map(|candidate| candidate.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            message: format!("failed to load any dynamic library candidate: {names}"),
        }
    }

    fn symbol(symbol: &CStr) -> Self {
        Self {
            message: format!(
                "failed to resolve dynamic symbol: {}",
                symbol.to_string_lossy()
            ),
        }
    }
}

impl fmt::Display for DynamicLibraryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DynamicLibraryError {}

enum HandleCloser<'a> {
    System,
    #[cfg(test)]
    Test(Box<dyn FnMut(*mut c_void) + 'a>),
    #[cfg(not(test))]
    _Lifetime(std::marker::PhantomData<&'a ()>),
}

#[cfg(test)]
type TestSymbolResolver<'a> = dyn Fn(*mut c_void, &CStr) -> *mut c_void + 'a;

enum SymbolResolver<'a> {
    System,
    #[cfg(test)]
    Test(Box<TestSymbolResolver<'a>>),
    #[cfg(not(test))]
    _Lifetime(std::marker::PhantomData<&'a ()>),
}

pub(crate) struct DynamicLibrary<'a> {
    handle: NonNull<c_void>,
    closer: HandleCloser<'a>,
    resolver: SymbolResolver<'a>,
}

impl DynamicLibrary<'static> {
    pub(crate) fn open_candidates(candidates: &[&CStr]) -> Result<Self, DynamicLibraryError> {
        for candidate in candidates {
            let handle = unsafe { dlopen(candidate.as_ptr(), RTLD_LAZY) };
            if let Some(handle) = NonNull::new(handle) {
                return Ok(Self {
                    handle,
                    closer: HandleCloser::System,
                    resolver: SymbolResolver::System,
                });
            }
        }

        Err(DynamicLibraryError::open(candidates))
    }
}

impl<'a> DynamicLibrary<'a> {
    pub(crate) unsafe fn symbol<T: Copy>(&self, symbol: &CStr) -> Result<T, DynamicLibraryError> {
        let address = match &self.resolver {
            SymbolResolver::System => unsafe { dlsym(self.handle.as_ptr(), symbol.as_ptr()) },
            #[cfg(test)]
            SymbolResolver::Test(resolve) => resolve(self.handle.as_ptr(), symbol),
            #[cfg(not(test))]
            SymbolResolver::_Lifetime(_) => unreachable!(),
        };
        let address = NonNull::new(address).ok_or_else(|| DynamicLibraryError::symbol(symbol))?;

        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        let raw = address.as_ptr();
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&raw) })
    }

    #[cfg(test)]
    fn from_raw_for_test(handle: NonNull<c_void>, close: impl FnMut(*mut c_void) + 'a) -> Self {
        Self {
            handle,
            closer: HandleCloser::Test(Box::new(close)),
            resolver: SymbolResolver::Test(Box::new(|_, _| std::ptr::null_mut())),
        }
    }
}

impl Drop for DynamicLibrary<'_> {
    fn drop(&mut self) {
        match &mut self.closer {
            HandleCloser::System => unsafe {
                dlclose(self.handle.as_ptr());
            },
            #[cfg(test)]
            HandleCloser::Test(close) => close(self.handle.as_ptr()),
            #[cfg(not(test))]
            HandleCloser::_Lifetime(_) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod symbol_tests {
    use super::*;
    use std::ffi::c_void;

    #[test]
    fn missing_symbol_error_names_the_symbol() {
        let library = DynamicLibrary::open_candidates(&[c"libX11.so.6", c"libX11.so"])
            .expect("test requires libX11");

        let error = unsafe {
            library
                .symbol::<unsafe extern "C" fn()>(c"vietc_symbol_that_does_not_exist")
                .expect_err("missing symbol must not resolve")
        };

        assert!(error
            .to_string()
            .contains("vietc_symbol_that_does_not_exist"));
    }

    #[test]
    fn failed_symbol_resolution_drops_the_open_handle() {
        let close_calls = std::cell::Cell::new(0);
        let handle = std::ptr::NonNull::dangling();

        {
            let library = DynamicLibrary::from_raw_for_test(handle, |_: *mut c_void| {
                close_calls.set(close_calls.get() + 1);
            });

            let result =
                unsafe { library.symbol::<unsafe extern "C" fn()>(c"missing_fixture_symbol") };
            assert!(result.is_err());
        }

        assert_eq!(close_calls.get(), 1);
    }
}
