mod camera;
mod capture;
mod dx_hooks;
mod falke;
mod game;
mod logger;
mod math;
mod replay;
mod thps4;
mod thug2;
mod ui;
use std::ffi::c_void;

use windows::Win32::{
    Foundation::HMODULE,
    System::{
        LibraryLoader::DisableThreadLibraryCalls,
        SystemServices::DLL_PROCESS_ATTACH,
        Threading::{CreateThread, THREAD_CREATION_FLAGS},
    },
};

// Main Entrypoint of our DLL.
unsafe extern "system" fn main_thread(_: *mut c_void) -> u32 {
    match std::panic::catch_unwind(falke::main) {
        Ok(Ok(())) => 0,
        Ok(Err(err)) => {
            log::error!("falke init failed: {err:#}");
            1
        }
        Err(panic) => {
            eprintln!("err: {panic:?}");
            log::error!("{panic:?}");
            1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "stdcall" fn DllMain(dll_module: HMODULE, reason: u32, _: *mut ()) -> bool {
    if reason == DLL_PROCESS_ATTACH {
        unsafe {
            let _ = DisableThreadLibraryCalls(dll_module);
            match CreateThread(
                None,
                0,
                Some(main_thread),
                None,
                THREAD_CREATION_FLAGS(0),
                None,
            ) {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("err: {err}")
                }
            }
        }
    }

    true
}
