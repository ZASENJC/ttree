//! Windows common dialogs. Runs on the UI thread so the owner stays modal and
//! Explorer's folder picker keeps native keyboard/accessibility behavior.
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM,
    FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

pub unsafe fn pick_model_folder(
    owner: super::HWND,
    current: &Path,
    title: &str,
) -> Result<Option<PathBuf>, String> {
    CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
        .ok()
        .map_err(|e| format!("无法初始化文件夹选择器：{e}"))?;
    let _apartment = Apartment;
    let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("无法创建文件夹选择器：{e}"))?;
    let result = (|| -> windows::core::Result<Option<PathBuf>> {
        dialog.SetOptions(
            dialog.GetOptions()?
                | FOS_PICKFOLDERS
                | FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                | FOS_NOCHANGEDIR,
        )?;
        let title = super::wide_null(title);
        dialog.SetTitle(PCWSTR(title.as_ptr()))?;
        dialog.SetOkButtonLabel(w!("选择此文件夹并下载"))?;
        // Do not create directories merely to open the picker: cancelling must
        // leave the model path and download state unchanged.
        if let Some(initial) = current.ancestors().find(|path| path.is_dir()) {
            let path: Vec<u16> = initial.as_os_str().encode_wide().chain(Some(0)).collect();
            if let Ok(folder) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(path.as_ptr()), None)
            {
                dialog.SetFolder(&folder)?;
            }
        }
        match dialog.Show(HWND(owner)) {
            Ok(()) => {}
            Err(e) if e.code().0 == 0x800704c7u32 as i32 => return Ok(None),
            Err(e) => return Err(e),
        }
        let folder = dialog.GetResult()?;
        let name = folder.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(path))
    })();
    result.map_err(|e| format!("选择模型下载文件夹失败：{e}"))
}

/// Confirmation belongs to the product flow: display the actual selected model
/// folder and keep No as the default before deleting owned model/runtime files.
pub unsafe fn confirm_remove_service(owner: super::HWND, models: &Path) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
    };
    let message=super::wide_null(&format!("将停止内置服务并删除：\n\n• 默认目录及以下目录中的内置 HY-MT1.5-1.8B 全部精度模型和下载断点：\n{}\n\n• TTREE 托管的推理引擎、依赖、下载包和日志。\n\n其他模型、文件以及翻译配置和聊天记录会保留。是否继续？",models.display()));
    MessageBoxW(
        HWND(owner),
        PCWSTR(message.as_ptr()),
        w!("删除内置模型服务"),
        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
    ) == IDYES
}
