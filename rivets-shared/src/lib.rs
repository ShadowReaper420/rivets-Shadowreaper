use anyhow::{bail, Result};
use cpp_demangle::Symbol;
use dirs::home_dir;
use pdb::{FallibleIterator, PDB};
use std::ffi::CString;
use std::net::TcpStream;
use std::path::Path;
use std::sync::Mutex;
use std::{collections::HashMap, fs::File};
use std::{ffi::CStr, path::PathBuf};
use tracing::info;
use undname::Flags;
use windows::core::PCSTR;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;

struct PDBCache {
    pdb: PDB<'static, File>,
    symbol_addresses: HashMap<String, u32>,
    base_address: u64,
}

impl PDBCache {
    fn new(pdb_path: &Path, module_name: &str) -> Result<Self> {
        let file = File::open(pdb_path)?;
        let pdb = PDB::open(file)?;
        let base_address = unsafe { get_dll_base_address(module_name)? };

        let mut cache = Self {
            pdb,
            symbol_addresses: HashMap::new(),
            base_address,
        };

        cache.build_symbol_map()?;

        Ok(cache)
    }

    fn build_symbol_map(&mut self) -> Result<()> {
        let symbol_table = self.pdb.global_symbols()?;
        let address_map = self.pdb.address_map()?;

        symbol_table
            .iter()
            .for_each(|symbol| match symbol.parse() {
                Ok(pdb::SymbolData::Public(data)) if data.function => {
                    let rva = data.offset.to_rva(&address_map).unwrap_or_default();
                    self.symbol_addresses
                        .insert(data.name.to_string().into(), rva.0);
                    Ok(())
                }
                Err(e) => Err(e),
                _ => Ok(()),
            })?;

        Ok(())
    }

    fn get_function_address(&self, function_name: &str) -> Option<u64> {
        self.symbol_addresses
            .get(function_name)
            .copied()
            .map(|x| self.base_address + u64::from(x))
    }
}

unsafe fn get_dll_base_address(module_name: &str) -> Result<u64> {
    let result = GetModuleHandleA(CString::new(module_name)?.as_pcstr());
    match result {
        Ok(handle) => Ok(handle.0 as u64),
        Err(err) => bail!(err),
    }
}

pub fn inject(function_name: &str, hook: unsafe fn(u64) -> Result<()>) -> Result<()> {
    let pdb_path = factorio_path("factorio.pdb")?;
    let pdb_cache = PDBCache::new(&pdb_path, "factorio.exe")?;

    let Some(address) = pdb_cache.get_function_address(function_name) else {
        bail!("Failed to find {function_name} address");
    };
    info!("{} address: {:#x}", function_name, address);

    unsafe { hook(address) }
}

pub fn start_stream() {
    let ip = "127.0.0.1:40267";
    let stream = TcpStream::connect(ip).unwrap_or_else(|_| {
        panic!("Could not establish stdout connection to rivets. Port {ip} is busy.")
    });
    tracing_subscriber::fmt::fmt()
        .with_writer(Mutex::new(stream))
        .init();
}

pub trait AsPcstr {
    fn as_pcstr(&self) -> PCSTR;
}

impl AsPcstr for CStr {
    fn as_pcstr(&self) -> PCSTR {
        PCSTR(self.as_ptr().cast())
    }
}

pub fn factorio_path(filename: &str) -> Result<PathBuf> {
    let factorio_path = home_dir();

    if let Some(mut path) = factorio_path {
        path.push(r"Documents\factorio\bin\x64\");
        path.push(filename);
        Ok(path)
    } else {
        bail!("Failed to find the user's home directory.")
    }
}

/// Attempts to demangle a mangled MSVC C++ symbol name. First tries MSVC demangling, then falls back to Itanium.
#[must_use]
pub fn demangle(mangled: &str) -> Option<String> {
    undname::demangle(mangled.into(), Flags::NO_ACCESS_SPECIFIER).map_or_else(
        |_| Symbol::new(mangled).ok().map(|x| x.to_string()),
        |x| Some(x.to_string()),
    )
}

/// Takes an unmangled C++ MSVC symbol name and returns the calling convention.
/// Fails if the calling convention is not one of cdecl, stdcall, fastcall, thiscall, or vectorcall.
#[must_use]
pub fn get_calling_convention(abi: &str) -> Option<syn::Abi> {
    Some(match abi {
        "__cdecl" => syn::parse_quote! { extern "C" },
        "__stdcall" => syn::parse_quote! { extern "stdcall" },
        "__fastcall" => syn::parse_quote! { extern "fastcall" },
        "__thiscall" => syn::parse_quote! { extern "thiscall" },
        "__vectorcall" => syn::parse_quote! { extern "vectorcall" },
        _ => return None,
    })
}
