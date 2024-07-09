use dll_syringe::{process::OwnedProcess, Syringe};
use std::io;
use std::net::TcpListener;
use windows::core::{PCSTR, PSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Threading::{
    CreateProcessA, ResumeThread, CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOA,
};

trait AsPcstr {
    fn as_pcstr(&self) -> PCSTR;
}

impl AsPcstr for &str {
    fn as_pcstr(&self) -> PCSTR {
        let null_terminated = format!("{}\0", self);
        PCSTR::from_raw(null_terminated.as_ptr())
    }
}

fn inject_dll(dll_name: &str) {
    println!("Injecting DLL into Factorio process...");
    let target_process =
        OwnedProcess::find_first_by_name("factorio").expect("Failed to find Factorio process.");
    let syringe = Syringe::for_process(target_process);
    let option = syringe.inject(dll_name);

    match option {
        Ok(_) => println!("DLL injected successfully."),
        Err(e) => panic!("Failed to inject DLL: {}", e),
    }
}

fn start_factorio(factorio_path: &str) -> Result<PROCESS_INFORMATION, String> {
    let mut startup_info: STARTUPINFOA = unsafe { std::mem::zeroed() };
    startup_info.cb = std::mem::size_of::<STARTUPINFOA>() as u32;
    let mut factorio_process_information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    startup_info.cb = std::mem::size_of::<STARTUPINFOA>() as u32;

    println!("Creating factorio.exe process...");
    let process_result = unsafe {
        CreateProcessA(
            factorio_path.as_pcstr(),
            PSTR::null(),
            None,
            None,
            false,
            CREATE_SUSPENDED,
            None,
            PCSTR::null(),
            &mut startup_info,
            &mut factorio_process_information,
        )
    };

    if let Err(err) = process_result {
        return Err(format!("Failed to create Factorio process: {}", err));
    }

    println!("Factorio process created successfully.");

    Ok(factorio_process_information)
}

unsafe fn get_dll_base_address(module_name: &str) -> Result<u64, String> {
    let result = GetModuleHandleA(module_name.as_pcstr());
    match result {
        Ok(handle) => Ok(handle.0 as u64),
        Err(err) => Err(format!("{}", err)),
    }
}

fn main() {
    let dll_path = r"target\debug\examplemod.dll";

    let listener = match TcpListener::bind("127.0.55.1:16337") {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "Failed to copy the Factorio output logs. Is rivets already running?\n{}",
                e
            );
            return;
        }
    };

    let factorio_path = r"C:\Users\zacha\Documents\factorio\bin\x64\factorio.exe";
    let factorio_process_information: PROCESS_INFORMATION;

    match start_factorio(factorio_path) {
        Ok(pi) => factorio_process_information = pi,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    }
    let process_handle = factorio_process_information.hProcess;

    inject_dll(&dll_path);

    let base_address = unsafe { get_dll_base_address("factorio.exe") }.unwrap();
    println!("Factorio base address: {:?}", base_address);

    unsafe {
        ResumeThread(factorio_process_information.hThread);
        CloseHandle(factorio_process_information.hThread).ok();
        CloseHandle(process_handle).ok();
    }

    // Duplicate the factorio stdout stream onto our own stdout.
    io::copy(
        &mut listener.incoming().next().unwrap().unwrap(),
        &mut io::stdout(),
    )
    .unwrap();
}
