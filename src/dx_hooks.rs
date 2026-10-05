use anyhow::{Context, Result};
use egui_d3d9::EguiDx9;
use retour::static_detour;
use std::{
    mem::transmute,
    sync::{Mutex, OnceLock},
};
use windows::core::Interface;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Direct3D9::{
                D3D_SDK_VERSION, D3DADAPTER_DEFAULT, D3DCREATE_SOFTWARE_VERTEXPROCESSING,
                D3DDEVTYPE_HAL, D3DDISPLAYMODE, D3DFORMAT, D3DPRESENT_PARAMETERS,
                D3DSWAPEFFECT_DISCARD, Direct3DCreate9, IDirect3DDevice9,
            },
            Gdi::RGNDATA,
        },
        UI::WindowsAndMessaging::{
            CallWindowProcW, GWLP_WNDPROC, GetDesktopWindow, SetWindowLongPtrA, WNDPROC,
        },
    },
    core::HRESULT,
};

use crate::{
    capture, game,
    ui::{UI_SYSTEM, ui_render_loop},
};

static_detour! {
    static PresentHook: unsafe extern "stdcall" fn(IDirect3DDevice9, *const RECT, *const RECT, HWND, *const RGNDATA) -> HRESULT;
    static ResetHook: unsafe extern "stdcall" fn(IDirect3DDevice9, *const D3DPRESENT_PARAMETERS) -> HRESULT;
    static EndSceneHook: unsafe extern "stdcall" fn(*mut IDirect3DDevice9) -> HRESULT;
}

struct AppCell(Mutex<EguiDx9<i32>>);
unsafe impl Sync for AppCell {}
unsafe impl Send for AppCell {}

static APP: OnceLock<AppCell> = OnceLock::new();
static OLD_WND_PROC: OnceLock<WNDPROC> = OnceLock::new();

pub unsafe fn install_hooks() -> Result<()> {
    log::debug!("installing d3d9 hooks");

    let device = get_d3d9_device().context("failed to obtain a temporary d3d9 device")?;
    macro_rules! install_d3d9 {
        ($hook:ident, $vtbl_field:ident, $detour:ident, $name:literal) => {{
            let addr = transmute(device.vtable().$vtbl_field);
            log::debug!("[addr] Direct3D9::{}: 0x{:x}", $name, addr as usize);
            $hook
                .initialize(addr, $detour)
                .context(concat!("failed to init d3d9::", $name, " hook"))?
                .enable()
                .context(concat!("failed to enable d3d9::", $name, " hook"))?;
        }};
    }
    install_d3d9!(PresentHook, Present, hk_present, "Present");
    install_d3d9!(EndSceneHook, EndScene, hk_endscene, "EndScene");
    install_d3d9!(ResetHook, Reset, hk_reset, "Reset");

    Ok(())
}

fn hk_present(
    dev: IDirect3DDevice9,
    source_rect: *const RECT,
    dest_rect: *const RECT,
    window: HWND,
    rgn_data: *const RGNDATA,
) -> HRESULT {
    unsafe {
        let cell = APP.get_or_init(|| {
            let mut params = std::mem::zeroed();
            dev.GetCreationParameters(&mut params).unwrap();
            let hwnd = params.hFocusWindow;

            let app = EguiDx9::init(&dev, hwnd, ui_render_loop, 0, true);
            let prev: WNDPROC = transmute(SetWindowLongPtrA(
                hwnd,
                GWLP_WNDPROC,
                hk_wnd_proc as *const () as _,
            ));
            let _ = OLD_WND_PROC.set(prev);
            AppCell(Mutex::new(app))
        });

        cell.0.lock().unwrap().present(&dev);

        PresentHook.call(dev, source_rect, dest_rect, window, rgn_data)
    }
}

fn hk_reset(
    dev: IDirect3DDevice9,
    presentation_parameters: *const D3DPRESENT_PARAMETERS,
) -> HRESULT {
    unsafe {
        if let Some(cell) = APP.get() {
            cell.0.lock().unwrap().pre_reset();
        }

        ResetHook.call(dev, presentation_parameters)
    }
}

unsafe extern "stdcall" fn hk_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(cell) = APP.get() {
        cell.0.lock().unwrap().wnd_proc(msg, wparam, lparam);
    }

    unsafe { CallWindowProcW(*OLD_WND_PROC.get().unwrap(), hwnd, msg, wparam, lparam) }
}

fn get_d3d9_device() -> Option<IDirect3DDevice9> {
    unsafe {
        let d9 = Direct3DCreate9(D3D_SDK_VERSION)
            .context("failed to create d3d")
            .ok()?;
        let d3d_display_mode = D3DDISPLAYMODE {
            Width: 0,
            Height: 0,
            RefreshRate: 0,
            Format: D3DFORMAT(0),
        };
        let mut present_params = D3DPRESENT_PARAMETERS {
            Windowed: windows::Win32::Foundation::BOOL(1),
            SwapEffect: D3DSWAPEFFECT_DISCARD,
            BackBufferFormat: d3d_display_mode.Format,
            ..core::mem::zeroed()
        };
        let mut device: Option<IDirect3DDevice9> = None;
        d9.CreateDevice(
            D3DADAPTER_DEFAULT,
            D3DDEVTYPE_HAL,
            GetDesktopWindow(),
            D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
            &mut present_params,
            &mut device,
        )
        .ok()?;

        device
    }
}

pub fn hk_endscene(device: *mut IDirect3DDevice9) -> HRESULT {
    let playback = UI_SYSTEM
        .get()
        .filter(|_| game::camera().is_some())
        .and_then(|ui_system| ui_system.lock().unwrap().update());

    unsafe { capture::on_endscene(playback.as_ref()) };

    unsafe { EndSceneHook.call(device) }
}
