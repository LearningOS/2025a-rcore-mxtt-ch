//! Process management syscalls
use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};
use crate::task::current_user_token;
use crate::timer::get_time_us;
use crate::mm::{translated_byte_buffer, VirtAddr, PTEFlags, PageTable};
use crate::config::PAGE_SIZE;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    // 获取当前时间（微秒）
    let us = get_time_us();
    let tv = TimeVal { sec: us / 1_000_000, usec: us % 1_000_000 };

    // 将结果写入用户空间地址（可能跨页）
    let token = current_user_token();
    let ptr = ts as *const u8;
    let len = core::mem::size_of::<TimeVal>();
    let mut bufs = translated_byte_buffer(token, ptr, len);
    let src_bytes = unsafe {
        core::slice::from_raw_parts((&tv as *const TimeVal) as *const u8, len)
    };
    let mut written = 0usize;
    for buf in bufs.iter_mut() {
        let to_copy = core::cmp::min(buf.len(), len - written);
        buf[..to_copy].copy_from_slice(&src_bytes[written..written + to_copy]);
        written += to_copy;
        if written >= len { break; }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");

    let token = current_user_token();
    let pt = PageTable::from_token(token);
    let va = VirtAddr::from(id);
    let vpn = va.floor();
    if let Some(pte) = pt.translate(vpn) {
        let flags = pte.flags();
        // 仅当对用户可见（U）并且具备对应的读/写权限时才允许
        match trace_request {
            0 => { // read
                if (flags & (PTEFlags::U | PTEFlags::R)) != (PTEFlags::U | PTEFlags::R) {
                    return -1;
                }
                let ppn = pte.ppn();
                let bytes = ppn.get_bytes_array();
                let byte = bytes[va.page_offset()];
                return byte as isize;
            }
            1 => { // write
                if (flags & (PTEFlags::U | PTEFlags::W)) != (PTEFlags::U | PTEFlags::W) {
                    return -1;
                }
                let ppn = pte.ppn();
                let bytes = ppn.get_bytes_array();
                bytes[va.page_offset()] = (data & 0xff) as u8;
                return 0;
            }
            _ => {
                return 0;
            }
        }
    } else {
        -1
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    // 校验 start 按页对齐
    if start % PAGE_SIZE != 0 { return -1; }
    // len 允许为 0
    if len == 0 { return 0; }
    // 校验 prot：仅低 3 位有效，且至少有一位
    if (prot & !0x7) != 0 { return -1; }
    if (prot & 0x7) == 0 { return -1; }

    // 计算页范围
    let start_va = VirtAddr::from(start);
    let npages = (len + PAGE_SIZE - 1) / PAGE_SIZE;

    // 将 prot 转换为页表标志，并加上 U
    let mut flags = PTEFlags::empty();
    if (prot & 0x1) != 0 { flags |= PTEFlags::R; }
    if (prot & 0x2) != 0 { flags |= PTEFlags::W; }
    if (prot & 0x4) != 0 { flags |= PTEFlags::X; }
    flags |= PTEFlags::U;

    // 冲突检测：区间内必须没有映射
    let token = current_user_token();
    let mut pt = PageTable::from_token(token);
    for i in 0..npages {
        let vpn = VirtAddr::from(start + i * PAGE_SIZE).floor();
        if let Some(pte) = pt.translate(vpn) {
            if pte.is_valid() { return -1; }
        }
    }

    // 映射：为每页分配物理页并设置标志
    for i in 0..npages {
        let vpn_map = VirtAddr::from(start + i * PAGE_SIZE).floor();
        if let Some(frame) = crate::mm::frame_alloc() {
            let ppn = frame.ppn;
            pt.map(vpn_map, ppn, flags);
        } else {
            return -1;
        }
    }

    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if start % PAGE_SIZE != 0 { return -1; }
    if len == 0 { return 0; }

    let start_va = VirtAddr::from(start);
    let npages = (len + PAGE_SIZE - 1) / PAGE_SIZE;
    let token = current_user_token();
    let mut pt = PageTable::from_token(token);

    // 预检查：所有页都必须已映射
    for i in 0..npages {
        let vpn = VirtAddr::from(start + i * PAGE_SIZE).floor();
        match pt.translate(vpn) {
            Some(pte) if pte.is_valid() => {}
            _ => return -1,
        }
    }

    // 取消映射
    for i in 0..npages {
        let vpn2 = VirtAddr::from(start + i * PAGE_SIZE).floor();
        pt.unmap(vpn2);
    }
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
