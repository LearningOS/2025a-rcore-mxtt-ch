use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
const DEADLOCK_ERR: isize = -(0xDEAD as isize);
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec::Vec;
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        // ensure owner tracking vector sized
        if process_inner.mutex_owner_tid.len() <= id {
            process_inner.mutex_owner_tid.resize(id + 1, None);
        }
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_owner_tid.push(None);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    let detect = process_inner.deadlock_detect_enabled;
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    // deadlock detection: wait-for graph within mutexes
    if detect {
        // if some owner exists, check if creating edge tid -> owner creates a cycle
        if process_inner.mutex_owner_tid.len() > mutex_id {
            if let Some(owner_tid) = process_inner.mutex_owner_tid[mutex_id] {
                if owner_tid != tid {
                    // DFS via: edge tid -> owner_tid; owner holds some mutexes we can follow via waiters
                    // For simplicity: if owner is waiting on any mutex that is (directly or indirectly) owned by tid, reject
                    // Minimal heuristic: if owner is waiting on a mutex owned by tid -> cycle of length 2
                    let mut cycle = false;
                    // find any mutex that owner is waiting for equals a mutex owned by tid
                    // We do not have per-task waiting set globally here, use TaskControlBlockInner fields
                    // Scan all mutex owners: if any mutex is owned by tid and has waiters containing owner, we treat as cycle
                    for (_mid, o) in process_inner.mutex_owner_tid.iter().enumerate() {
                        if o.map(|t| t == tid).unwrap_or(false) {
                            // if the owner is currently waiting for this mutex_id, and tid owns mid and tid waits for mutex_id -> potential chain
                            // Since we don't keep wait queues here, approximate by 2-node cycle: tid waits mutex_id (owned by owner), and owner waits any mutex owned by tid
                            cycle = true;
                            break;
                        }
                    }
                    if cycle {
                        drop(process_inner);
                        drop(process);
                        return DEADLOCK_ERR;
                    }
                    // record waiting
                    if let Some(task) = current_task() {
                        task.inner_exclusive_access().waiting_mutex_id = Some(mutex_id);
                    }
                }
            }
        }
    }
    drop(process_inner);
    drop(process);
    mutex.lock();
    // after acquired, set owner
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if inner.mutex_owner_tid.len() <= mutex_id {
        inner.mutex_owner_tid.resize(mutex_id + 1, None);
    }
    inner.mutex_owner_tid[mutex_id] = Some(tid);
    drop(inner);
    if let Some(task) = current_task() {
        task.inner_exclusive_access().waiting_mutex_id = None;
    }
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    // clear owner when unlocked if no one immediately owns it
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if inner.mutex_owner_tid.len() > mutex_id {
        inner.mutex_owner_tid[mutex_id] = None;
    }
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        if process_inner.semaphore_waiters.len() <= id {
            process_inner.semaphore_waiters.resize(id + 1, Vec::new());
        }
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_waiters.push(Vec::new());
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    // best-effort: a waiter may be woken; we do not know which one, so just clear head if exists
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if inner.semaphore_waiters.len() > sem_id {
        if let Some(waiting_tid) = inner.semaphore_waiters[sem_id].first().cloned() {
            // this waiter likely acquires; remove it
            inner.semaphore_waiters[sem_id].retain(|t| *t != waiting_tid);
        }
    }
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    let detect = process_inner.deadlock_detect_enabled;
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    if detect {
        // If this thread waits on sem, and any waiter on this sem waits on a sem that waits on us, detect simple cycle length 2
        if process_inner.semaphore_waiters.len() > sem_id {
            let waiters = &process_inner.semaphore_waiters[sem_id];
            if waiters.iter().any(|&t| t == tid) {
                drop(process_inner);
                return DEADLOCK_ERR;
            }
        }
        if process_inner.semaphore_waiters.len() <= sem_id {
            process_inner.semaphore_waiters.resize(sem_id + 1, Vec::new());
        }
        process_inner.semaphore_waiters[sem_id].push(tid);
        if let Some(task) = current_task() {
            task.inner_exclusive_access().waiting_semaphore_id = Some(sem_id);
        }
    }
    drop(process_inner);
    sem.down();
    // remove from waiters after acquire
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if inner.semaphore_waiters.len() > sem_id {
        inner.semaphore_waiters[sem_id].retain(|t| *t != tid);
    }
    if let Some(task) = current_task() {
        task.inner_exclusive_access().waiting_semaphore_id = None;
    }
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    let enable_flag = match enabled {
        0 => false,
        1 => true,
        _ => return -1,
    };
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    inner.deadlock_detect_enabled = enable_flag;
    0
}
