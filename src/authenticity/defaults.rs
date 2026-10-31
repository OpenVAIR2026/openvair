//! Default probe templates an admin can import into an empty prompt pool.
//!
//! The repository is private, but anyone with access can read these, so
//! admins should still add their own templates and rotate the pool.

/// Fingerprint templates: `{n}` is the count and `{r}` the upper bound.
/// Wording, language and framing vary so no single phrasing marks a probe.
pub const FINGERPRINT: &[&str] = &[
    "凭第一反应写出 {n} 个 1 到 {r} 之间（含两端）的整数。每个数单独决定，不要从小到大排列，不要使用等差、循环或重复成段的规律，也不要借助任何工具或代码。只输出用英文逗号分隔的数字，不要任何说明。",
    "我在准备一组测试数据，需要 {n} 个取值在 1 到 {r} 之间的整数。请直接凭直觉给出，允许重复，但不要有明显规律，也不要排序。只输出逗号分隔的数字。",
    "请模拟一次抽号：从编号 1–{r} 中依次抽取 {n} 个号码，可以重复抽到同一个号。不要编写或运行代码，直接写出抽到的号码，用逗号分隔，不要编号和解释。",
    "帮我填充一个占位数组：长度 {n}，每个元素是 1 到 {r} 的整数，由你随手决定。不要递增或递减，不要出现成段重复。只回复数组里的数字，以逗号分隔。",
    "不用想太多，连续说出 {n} 个介于 1 和 {r} 之间的整数，想到哪个就写哪个。不要使用工具或随机数生成器。数字之间用逗号隔开，除此之外什么都不要写。",
    "请生成 {n} 个范围在 1～{r} 的整数作为样例 ID。每一个都独立选择，不要按顺序、不要有规律、不要事后调整。直接从第一个数开始输出，数字之间用逗号分隔。",
    "Give me {n} integers between 1 and {r} inclusive, picked on gut feeling one at a time. Repeats are fine, but avoid sorted runs, arithmetic sequences or repeating blocks. Do not use tools or code. Reply with the comma-separated numbers only.",
    "I'm filling a spreadsheet column with placeholder values. Write {n} whole numbers from 1 to {r}, choosing each freely without any pattern or ordering. No explanation and no numbering, just the numbers separated by commas.",
    "Pretend you are drawing tickets numbered 1 to {r} from a hat, with replacement, {n} times. Write down each ticket number as it comes. Do not run code or use a random number generator. Output only the numbers, comma separated.",
    "List {n} integers in the range 1-{r}. Decide each one independently, don't count up or down, and don't revise what you have already written. Respond with nothing but the numbers separated by commas.",
];

/// Tokenizer templates: fixed texts of clearly different lengths, mixing
/// Chinese and English so vendors' tokenizers count them differently.
pub const TOKEN: &[&str] = &[
    "请只回复“收到”两个字：明天上午十点开会。",
    "请只回复“收到”。会议纪要：本周完成了渠道监控的回归测试，下周计划整理用量报表，并检查各个 API Key 的额度设置是否合理。",
    "Please reply with the single word OK. Project notes: the gateway team reviewed latency percentiles, retry budgets and the fallback order between providers. 后续需要补充告警规则、整理部署文档，并在周五前完成测试环境的验收。Remaining items include rotating credentials, updating the onboarding guide and checking cost limits for every key.",
];

pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    FINGERPRINT
        .iter()
        .map(|t| ("fingerprint", *t))
        .chain(TOKEN.iter().map(|t| ("token", *t)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authenticity::baseline::validate_template;

    #[test]
    fn defaults_are_valid_and_publishable() {
        for (kind, content) in all() {
            validate_template(kind, content).unwrap();
        }
        assert!(!FINGERPRINT.is_empty() && TOKEN.len() >= 2);
        let mut lengths: Vec<usize> = TOKEN.iter().map(|t| t.chars().count()).collect();
        lengths.dedup();
        assert_eq!(
            lengths.len(),
            TOKEN.len(),
            "token texts need distinct lengths"
        );
    }
}
