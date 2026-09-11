//! 确定性 JSON 规范化（存档信封 CRC 与指纹门禁共享）。
//! 递归键排序消除 HashMap 序列化键序噪声；Vec 顺序仍视为语义（journal/decisions 有序）。

use serde::Serialize;

/// 状态值 → 键排序后的规范化 JSON 字节（对象键递归字典序）。
/// 键排序保证 `HashMap` 类字段（archive/players 等）任意迭代序下输出一致。
/// 注意：数组顺序**不**重排——Vec 顺序是状态语义（事件流/决策日志有序）。
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    fn sort_json(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for child in map.values_mut() {
                    sort_json(child);
                }
                let mut entries: Vec<(String, serde_json::Value)> =
                    std::mem::take(map).into_iter().collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                for (key, child) in entries {
                    map.insert(key, child);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(sort_json),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(value).map_err(|e| format!("状态序列化失败：{e}"))?;
    sort_json(&mut v);
    serde_json::to_vec(&v).map_err(|e| format!("状态编码失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HashMap 迭代序噪声消除：同内容不同插入序 → 同字节。
    #[test]
    fn map_key_order_is_normalized() {
        let a: std::collections::BTreeMap<&str, u32> = [("x", 1), ("y", 2)].into_iter().collect();
        let b: std::collections::BTreeMap<&str, u32> = [("y", 2), ("x", 1)].into_iter().collect();
        assert_eq!(
            canonical_json_bytes(&a).unwrap(),
            canonical_json_bytes(&b).unwrap()
        );
    }

    /// Vec 顺序是语义：交换数组元素必须改变输出。
    #[test]
    fn vec_order_is_semantic() {
        assert_ne!(
            canonical_json_bytes(&vec![1, 2]).unwrap(),
            canonical_json_bytes(&vec![2, 1]).unwrap()
        );
    }
}
