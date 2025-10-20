//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;

/// BIG_STRIDE value for stride scheduling algorithm
const BIG_STRIDE: usize = 10000;

///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }
        
        // 使用stride调度算法找到stride最小的任务
        let mut min_stride_index = 0;
        let mut min_stride = usize::MAX;
        
        for (i, task) in self.ready_queue.iter().enumerate() {
            let task_inner = task.inner_exclusive_access();
            if task_inner.stride < min_stride {
                min_stride = task_inner.stride;
                min_stride_index = i;
            }
            drop(task_inner);
        }
        
        // 取出stride最小的任务
        let task = self.ready_queue.remove(min_stride_index).unwrap();
        
        // 更新该任务的stride值
        let mut task_inner = task.inner_exclusive_access();
        let pass = BIG_STRIDE / task_inner.priority;
        task_inner.stride += pass;
        drop(task_inner);
        
        Some(task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
