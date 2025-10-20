## stride 算法深入

stride 算法原理非常简单，但是有一个比较大的问题。例如两个 pass = 10 的进程，使用 8bit 无符号整形储存 stride， p1.stride = 255, p2.stride = 250，在 p2 执行一个时间片后，理论上下一次应该 p1 执行。

- 实际情况是轮到 p1 执行吗？为什么？

  不是。在例子中，两个进程的 pass 值均为 10，p1.stride = 255，p2.stride = 250。当 p2 执行一个时间片后，其 stride 增加 pass 值 10，得到 260。由于使用 8 位无符号整数存储，260 会溢出，模 256 后值为 4。因此，p2 的新 stride 为 4。此时，比较 p1.stride = 255 和 p2.stride = 4，数值上 4 < 255，因此调度器会选择 p2 再次执行，而不是 p1。这是因为溢出导致 p2 的 stride 值变小，从而被误认为应该优先调度。

我们之前要求进程优先级 >= 2 其实就是为了解决这个问题。可以证明， **在不考虑溢出的情况下** , 在进程优先级全部 >= 2 的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。

- 为什么？尝试简单说明（不要求严格证明）。

  当所有进程的优先级 ≥ 2 时，每个进程的 pass 值（pass = BigStride / priority）满足 pass ≤ BigStride / 2。在调度过程中，每次总是选择 stride 值最小的进程执行，并将其 stride 增加 pass。由于 pass ≤ BigStride / 2，任何进程 stride 值的增加量都不会超过 BigStride / 2。因此，在任意时刻，所有进程的 stride 值之间的最大差值不会超过 BigStride / 2。具体来说：

  - 从初始状态（所有 stride 为 0）开始，差值為 0。
  - 每次调度后，被调度进程的 stride 增加最多 BigStride / 2，而其他进程的 stride 不变，因此最大差值最多增加 BigStride / 2。
  - 但由于总是选择最小 stride 进程执行，增加后的 stride 可能不再是最小值，实际差值会被限制在 BigStride / 2 以内。

  因此，在不考虑溢出的情况下，STRIDE_MAX – STRIDE_MIN ≤ BigStride / 2。

- 已知以上结论，**考虑溢出的情况下**，可以为 Stride 设计特别的比较器，让 BinaryHeap<Stride> 的 pop 方法能返回真正最小的 Stride。补全下列代码中的 `partial_cmp` 函数，假设两个 Stride 永远不会相等。

  在考虑溢出的情况下，为了正确比较 Stride 值，需要利用有符号差来判断。通过计算两个 Stride 值的无符号包装差（wrapping subtraction），然后将结果解释为有符号数，如果结果为负，则第一个值较小；否则第一个值较大。

  ```
  use core::cmp::Ordering;
  
  struct Stride(u64);
  
  impl PartialOrd for Stride {
      fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
          let diff = self.0.wrapping_sub(other.0) as i64;
          if diff < 0 {
              Some(Ordering::Less)
          } else {
              Some(Ordering::Greater)
          }
      }
  }
  
  impl PartialEq for Stride {
      fn eq(&self, other: &Self) -> bool {
          false
      }
  }
  ```

  



TIPS: 使用 8 bits 存储 stride, BigStride = 255, 则: `(125 < 255) == false`, `(129 < 255) == true`.