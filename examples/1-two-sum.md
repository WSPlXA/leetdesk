---
problem_id: 1
tags: [哈希表, 面试复习]
---

# 两数之和：我的做法讲解

## 思路

每次遇到一个数字，查找之前是否出现过它的补数。

## 为什么正确

遍历到下标 i 时，哈希表只记录 i 之前的元素，所以不会重复使用当前元素。

## Go 实现

```go
func twoSum(nums []int, target int) []int {
    seen := make(map[int]int)
    for i, n := range nums {
        if j, ok := seen[target-n]; ok {
            return []int{j, i}
        }
        seen[n] = i
    }
    return nil
}
```

## 复杂度

| 指标 | 复杂度 |
| --- | --- |
| 时间 | O(n) |
| 空间 | O(n) |

## 易错点

下标 0 是合法的值。应使用 Go map 查找的第二个返回值判断元素是否存在。
