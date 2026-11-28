//! 导入角色匹配：名称数据与程序逻辑分离，构建时嵌入别名表。

use pinyin::ToPinyin;
use std::collections::HashMap;
use std::sync::OnceLock;

type Aliases = HashMap<String, HashMap<String, Vec<String>>>;

fn aliases() -> &'static Aliases {
    static DATA: OnceLock<Aliases> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../character-aliases.json")).unwrap_or_default())
}

fn normalize(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

fn pinyin(text: &str) -> String {
    text.chars().map(|c| match c.to_pinyin() {
        Some(p) => p.plain().to_owned(),
        None => c.to_string(),
    }).collect()
}

/// 在当前游戏已有角色中匹配名称；多个角色命中时不自动选择。
pub fn guess(game_id: &str, candidates: &[String], text: &str) -> String {
    let input = normalize(text);
    let game = aliases().get(game_id);
    let mut matched: Option<&str> = None;
    for candidate in candidates {
        let mut names = vec![candidate.clone(), pinyin(candidate)];
        if let Some(extra) = game.and_then(|g| g.get(candidate)) {
            names.extend(extra.iter().cloned());
        }
        let hit = names.iter().any(|name| {
            let key = normalize(name);
            let valid = key.chars().count() >= 2 || (key.chars().count() == 1 && key.chars().any(|c| !c.is_ascii()));
            valid && input.contains(&key)
        });
        if hit {
            if matched.is_some_and(|existing| existing != candidate) {
                return String::new();
            }
            matched = Some(candidate);
        }
    }
    matched.unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_match_inside_filename_and_normalize() {
        let names = vec!["陈千语".to_owned(), "余烬".to_owned()];
        assert_eq!(guess("endfield", &names, "123chenada"), "陈千语");
        assert_eq!(guess("endfield", &names, "CHEN_QIANYU"), "陈千语");
        assert_eq!(guess("endfield", &names, "ember_mod"), "余烬");
        assert_eq!(guess("endfield", &names, "yujin_mod"), "余烬");
    }

    #[test]
    fn ambiguous_and_wrong_game_do_not_choose() {
        let names = vec!["陈千语".to_owned(), "余烬".to_owned()];
        assert_eq!(guess("endfield", &names, "chen_ember"), "");
        assert_eq!(guess("GenshinImpact", &names, "ember"), "");
    }

    #[test]
    fn data_is_valid_and_custom_names_work() {
        assert!(serde_json::from_str::<Aliases>(include_str!("../character-aliases.json")).is_ok());
        let names = vec!["自定义角色".to_owned()];
        assert_eq!(guess("endfield", &names, "自定义角色_mod"), "自定义角色");
    }
}
