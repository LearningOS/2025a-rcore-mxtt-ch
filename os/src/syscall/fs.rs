//! File and filesystem-related syscalls
use crate::drivers::BLOCK_DEVICE;
use crate::fs::{open_file, OpenFlags, OSInode, Stat, StatMode, File};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};
use easy_fs::{EasyFileSystem};
use alloc::sync::Arc;

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!("kernel:pid[{}] sys_fstat", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        drop(inner);
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        // 由于所有通过 sys_open 打开的文件都是 Arc<OSInode> 类型，
        // 我们可以不安全地将 Arc<dyn File> 转换为 Arc<OSInode>
        let os_inode = unsafe {
            let file_ptr = file as *const Arc<dyn File + Send + Sync>;
            &*(file_ptr as *const Arc<OSInode>)
        };
        
        // 获取 inode ID 和链接计数
        let ino = os_inode.get_inode_id();
        let nlink = os_inode.get_nlink();
        // 判断是目录还是普通文件
        let mode = if os_inode.is_dir() {
            StatMode::DIR
        } else {
            StatMode::FILE
        };
        
        drop(inner);
        
        let token = current_user_token();
        let stat_mut = crate::mm::translated_refmut(token, st);
        // 填充 stat 结构体
        stat_mut.dev = 0;
        stat_mut.ino = ino;
        stat_mut.mode = mode;
        stat_mut.nlink = nlink;
        stat_mut.pad = [0; 7];
        0
    } else {
        drop(inner);
        -1
    }
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_linkat", current_task().unwrap().pid.0);
    let _task = current_task().unwrap();
    let token = current_user_token();
    let old_path = translated_str(token, old_name);
    let new_path = translated_str(token, new_name);
    
    // 检查新旧路径是否相同
    if old_path == new_path {
        return -1;
    }
    
    // 获取根 inode
    let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));
    
    // 查找旧文件
    if let Some(old_file_inode) = root_inode.find(old_path.as_str()) {
        // 检查新文件是否已存在
        if root_inode.find(new_path.as_str()).is_some() {
            return -1;
        }
        
        // 创建硬链接
        if old_file_inode.link(root_inode.clone(), new_path.as_str()) {
            0
        } else {
            -1
        }
    } else {
        -1
    }
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(name: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_unlinkat", current_task().unwrap().pid.0);
    let _task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, name);
    
    // 获取根 inode
    let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));
    
    // 删除目录项
    if let Some(inode_id) = root_inode.unlink_entry(path.as_str()) {
        // 获取 inode 并减少链接计数
        let file_inode = Arc::new(EasyFileSystem::get_inode_by_id(&efs, inode_id));
        let nlink = file_inode.dec_nlink();
        
        // 如果链接计数变为 0，则删除文件
        if nlink == 0 {
            // 获取新的引用以清除文件
            let file_inode = Arc::new(EasyFileSystem::get_inode_by_id(&efs, inode_id));
            file_inode.clear();
        }
        0
    } else {
        -1
    }
}
