//! Dump an app's AX menu tree: titles, cmd chars, modifiers, roles.
//! Usage: cargo run --example axdump -- <pid>
//! Diagnostic for press_menu_cmd_char — shows what the AX walk actually sees.

#[cfg(target_os = "macos")]
fn main() {
    let pid: i32 = std::env::args().nth(1).expect("usage: axdump <pid>").parse().unwrap();
    unsafe { dump(pid) };
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macOS only");
}

#[cfg(target_os = "macos")]
unsafe fn dump(pid: i32) {
    use std::ffi::c_void;
    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(e: CFTypeRef, a: CFStringRef, v: *mut CFTypeRef) -> i32;
        fn AXUIElementCopyAttributeNames(e: CFTypeRef, names: *mut CFTypeRef) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(a: *const c_void, s: *const i8, e: u32) -> CFStringRef;
        fn CFStringGetCString(s: CFStringRef, b: *mut i8, l: CFIndex, e: u32) -> u8;
        fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> *const c_void;
        fn CFNumberGetValue(n: CFTypeRef, t: CFIndex, v: *mut c_void) -> u8;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFCopyTypeIDDescription(id: usize) -> CFStringRef;
    }

    const UTF8: u32 = 0x0800_0100;
    let cf = |s: &str| -> CFStringRef {
        let c = std::ffi::CString::new(s).unwrap();
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
    };
    let rs = |v: CFTypeRef| -> String {
        let mut buf = [0i8; 256];
        if unsafe { CFStringGetCString(v, buf.as_mut_ptr(), 256, UTF8) } != 0 {
            unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
        } else {
            "<?>".into()
        }
    };
    let attr = |e: CFTypeRef, name: &str| -> Option<CFTypeRef> {
        let mut v: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(e, cf(name), &mut v) };
        (err == 0 && !v.is_null()).then_some(v)
    };

    unsafe {
        println!("AXIsProcessTrusted = {}", AXIsProcessTrusted());
        let app = AXUIElementCreateApplication(pid);
        let Some(menubar) = attr(app, "AXMenuBar") else {
            println!("NO AXMenuBar");
            return;
        };
        let Some(bar_items) = attr(menubar, "AXChildren") else {
            println!("menubar has NO AXChildren");
            return;
        };
        let n = CFArrayGetCount(bar_items);
        println!("menu bar: {n} top-level items");
        for i in 0..n {
            let bar_item = CFArrayGetValueAtIndex(bar_items, i);
            let title = attr(bar_item, "AXTitle").map(&rs).unwrap_or_default();
            let role = attr(bar_item, "AXRole").map(&rs).unwrap_or_default();
            println!("[{i}] '{title}' ({role})");
            let Some(menus) = attr(bar_item, "AXChildren") else {
                println!("      <no children>");
                continue;
            };
            for j in 0..CFArrayGetCount(menus) {
                let menu = CFArrayGetValueAtIndex(menus, j);
                let mrole = attr(menu, "AXRole").map(&rs).unwrap_or_default();
                let Some(items) = attr(menu, "AXChildren") else {
                    println!("      menu[{j}] ({mrole}) <no children>");
                    continue;
                };
                let count = CFArrayGetCount(items);
                println!("      menu[{j}] ({mrole}) {count} items");
                for k in 0..count {
                    let item = CFArrayGetValueAtIndex(items, k);
                    let t = attr(item, "AXTitle").map(&rs).unwrap_or_default();
                    let c = attr(item, "AXMenuItemCmdChar").map(&rs).unwrap_or_default();
                    let m = attr(item, "AXMenuItemCmdModifiers").map(|v| {
                        let mut out: i32 = -1;
                        CFNumberGetValue(v, 3, &mut out as *mut i32 as *mut c_void);
                        out.to_string()
                    }).unwrap_or_default();
                    let vk = attr(item, "AXMenuItemCmdVirtualKey").map(|v| {
                        let mut out: i32 = -1;
                        CFNumberGetValue(v, 3, &mut out as *mut i32 as *mut c_void);
                        out.to_string()
                    }).unwrap_or_default();
                    if !t.is_empty() || !c.is_empty() {
                        println!("        [{k}] '{t}' cmdChar='{c}' mods={m} vk={vk}");
                    }
                }
            }
        }
        // Show available attribute names on the app for reference
        let mut names: CFTypeRef = std::ptr::null();
        if AXUIElementCopyAttributeNames(app, &mut names) == 0 && !names.is_null() {
            let n = CFArrayGetCount(names);
            let mut list = Vec::new();
            for i in 0..n {
                let v = CFArrayGetValueAtIndex(names, i);
                if CFGetTypeID(v) == { let d = CFCopyTypeIDDescription(CFGetTypeID(v)); let _ = d; CFGetTypeID(v) } {
                    list.push(rs(v));
                }
            }
            println!("app attributes: {list:?}");
        }
    }
}
