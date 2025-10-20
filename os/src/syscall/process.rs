//! Process management syscalls
use alloc::sync::Arc;

use crate::{
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str, translated_byte_buffer, VirtAddr, PTEFlags, PageTable},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskControlBlock,
    },
    timer::get_time_us,
    config::PAGE_SIZE,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
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

/// YOUR JOB: Implement mmap.
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


/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if start % PAGE_SIZE != 0 { return -1; }
    if len == 0 { return 0; }

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
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
pub fn sys_spawn(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        // 直接创建新任务
        let new_task = Arc::new(TaskControlBlock::new(data));
        let new_pid = new_task.pid.0;
        
        // 设置父子关系
        let current = current_task().unwrap();
        let mut new_task_inner = new_task.inner_exclusive_access();
        new_task_inner.parent = Some(Arc::downgrade(&current));
        drop(new_task_inner);
        
        let mut current_inner = current.inner_exclusive_access();
        current_inner.children.push(new_task.clone());
        drop(current_inner);
        
        // 将新任务添加到调度器
        add_task(new_task);
        
        // 返回子进程ID
        new_pid as isize
    } else {
        // 无效的文件名
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority prio: {}",
        current_task().unwrap().pid.0,
        prio
    );
    
    // 检查优先级是否合法（>= 2）
    if prio < 2 {
        return -1;
    }
    
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.priority = prio as usize;
    drop(inner);
    
    prio
}
