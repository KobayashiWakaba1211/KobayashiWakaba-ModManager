//! 各游戏固化的角色文件夹名（写到程序里，切换游戏时按需创建）。
//! 来源：各游戏 Mods 目录现存的真实角色文件夹（扫描逻辑认定：排除 DISABLED_* / DO NOT MODIFY_* 管理目录）。

/// game id -> 该游戏全部角色文件夹名
pub const BUILTIN_CHARACTERS: &[(&str, &[&str])] = &[
    ("endfield", &[
        "NPC", "Other Misc", "UI", "伊冯", "余烬", "佩丽卡",
        "其他", "别礼", "功能", "卡契尔", "卡繆", "噗切娜",
        "埃特拉", "大世界", "大潘", "女主", "安塔尔", "庄方宜",
        "弧光", "弭弗", "提弗洛斯", "明河", "昼雪", "梨诺",
        "武器", "汤汤", "洁尔佩塔", "洛茜", "狼卫", "男主",
        "祀", "秋栗", "艾尔黛拉", "艾维文娜", "莱万汀", "萤石",
        "诀", "赛希", "阿列什", "陈千语", "骏卫", "黎风",
    ]), // 42 个
    ("starrail", &[
        "Archer", "NPC", "Saber", "UI", "万敌", "三月七-存护",
        "三月七-巡猎", "不死途", "丹恒", "丹恒-腾荒", "丹恒-饮月", "乱破",
        "云璃", "佩拉", "停云", "光锥", "克拉拉", "其他",
        "刃", "刃-千治", "刻律德菈", "功能", "加拉赫", "卡芙卡",
        "卢卡", "大世界", "大丽花", "大黑塔", "女主", "姬子",
        "娜塔莎", "寒鸦", "布洛妮娅", "希儿", "希露瓦", "彦卿",
        "忘归人", "托帕&账账", "昔涟", "星期日", "景元", "杰帕德",
        "桂乃芬", "桑博", "椒丘", "波提欧", "流萤", "海瑟音",
        "火花", "灵砂", "爻光", "玲可", "瓦尔特", "男主",
        "白厄", "白露", "真珠", "真理医生", "知更鸟", "砂金",
        "符玄", "米沙", "素裳", "绯英", "缇宝", "罗刹",
        "翡翠", "艾丝妲", "花火", "藿藿", "虎克", "貊泽",
        "赛飞儿", "远坂凛", "遐蝶", "那刻夏", "银枝", "银狼",
        "银狼LV.999", "镜流", "长夜月", "阮梅", "阿兰", "阿格莱雅",
        "雪衣", "青雀", "风堇", "飞霄", "驭空", "黄泉",
        "黑塔", "黑天鹅",
    ]), // 92 个
    ("wuthering", &[
        "NPC", "UI", "丹瑾", "丽贝卡", "仇远", "今汐",
        "其他", "凌阳", "功能", "千咲", "卜灵", "卡卡罗",
        "卡提希娅", "吟霖", "嘉贝莉娜", "坎特蕾拉", "夏空", "大世界",
        "奥古斯塔", "女主", "守岸人", "安可", "尤诺", "布兰特",
        "希格莉卡", "弗洛洛", "心", "忌炎", "折枝", "散华",
        "桃祈", "椿", "武器", "洛可可", "洛瑟菈", "清宵",
        "渊武", "滑翔翼", "灯灯", "炽霞", "爱弥斯", "珂莱塔",
        "琳奈", "男主", "白芷", "相里要", "秋水", "秧秧",
        "秧秧·玄翎", "穗穗", "绯雪", "维里奈", "莫宁", "莫特斐",
        "菲比", "赞妮", "载具", "达妮娅", "釉瑚", "鉴心",
        "锁暝", "长离", "陆·赫斯", "露帕", "露西",
    ]), // 65 个
    ("zenless", &[
        "11号", "NPC", "UI", "丽娜", "仪玄", "伊德海莉",
        "伊芙琳", "佩洛伊斯", "凯撒", "功能", "千夏", "南宫羽",
        "卢西娅", "可琳", "叶瞬光", "大世界", "奥菲丝&鬼火", "女主",
        "妮可", "安东", "安比", "希希芙", "希格莉德", "席德",
        "悠真", "扳机", "普罗米娅", "本", "朱鸢", "柏妮思",
        "柚叶", "柳", "格莉丝", "橘福福", "武器", "比利",
        "波可娜", "派派", "潘引壶", "照", "爱丽丝", "爱芮",
        "猫又", "珂蕾妲", "琉音", "男主", "真斗", "简",
        "维琳娜", "耀嘉音", "般岳", "艾莲", "苍角", "莱卡恩",
        "莱特", "蕾米埃尔", "薇薇安", "诺姆", "赛斯", "雅",
        "雨果", "零号·安比", "露西", "青衣",
    ]), // 64 个
    ("GenshinImpact", &[
        "Characters", "Objects", "Other", "Prune", "UI", "Uncategorized",
        "七七", "丝柯克", "丽莎", "久岐忍", "九条裟罗", "云堇",
        "伊安珊", "伊涅芙", "优菈", "克洛琳德", "八重神子", "兹白",
        "凝光", "刻晴", "北斗", "千织", "卡齐娜", "可莉",
        "哥伦比娅", "坎蒂丝", "埃洛伊", "夏沃蕾", "夏洛蒂", "多莉",
        "夜兰", "奈芙尔", "奥黛塔", "妮露", "娜维娅", "安柏",
        "宵宫", "尼可", "希格雯", "希诺宁", "恰斯卡", "早柚",
        "柯莱", "桑多涅", "梦见月瑞希", "沃雅妮莎", "派蒙", "流浪者",
        "烟绯", "爱可菲", "玛拉妮", "玛薇卡", "珊瑚宫心海", "珐露珊",
        "琳妮特", "琴", "瑶瑶", "瓦雷莎", "甘雨", "申鹤",
        "砂糖", "神里绫华", "纳西妲", "绮良良", "罗莎莉亚", "胡桃",
        "艾梅莉埃", "芙宁娜", "芭芭拉", "茜特菈莉", "荧", "莉奈娅",
        "莫娜", "莱依拉", "菈乌玛", "菲谢尔", "蓝砚", "薇斯纳",
        "行秋", "诺艾尔", "辛焱", "迪卢克", "迪奥娜", "迪希雅",
        "重云", "钟离", "闲云", "阿蕾奇诺", "雅珂达", "雷电将军",
        "香菱",
    ]), // 91 个
];

/// 取某游戏固化的角色名列表（非内置游戏返回 None）
pub fn builtin_names(game_id: &str) -> Option<&'static [&'static str]> {
    BUILTIN_CHARACTERS
        .iter()
        .find(|(id, _)| *id == game_id)
        .map(|(_, names)| *names)
}

/// 在 mods_path 下按固化名单检查角色文件夹：存在则直接读取（保留），缺失则创建。
/// 返回 (新建数, 已存在数)。路径不含 mods 或该游戏无固化名单时返回 (0, 0)。
pub fn ensure_character_dirs(game_id: &str, mods_path: &str) -> Result<(usize, usize), String> {
    if !crate::config::is_mods_path(mods_path) {
        return Ok((0, 0));
    }
    let names = match builtin_names(game_id) {
        Some(n) => n,
        None => return Ok((0, 0)),
    };
    let root = std::path::Path::new(mods_path);
    let mut created = 0usize;
    let mut existed = 0usize;
    for name in names {
        let dir = root.join(name);
        if dir.is_dir() {
            existed += 1;
        } else {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("创建角色目录「{name}」失败: {e}"))?;
            created += 1;
        }
    }
    Ok((created, existed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_cover_five_games() {
        for id in ["endfield", "starrail", "wuthering", "zenless", "GenshinImpact"] {
            let names = builtin_names(id).unwrap();
            assert!(names.len() > 20, "{id} 角色数异常: {}", names.len());
            assert!(names.iter().all(|n| !n.is_empty()), "{id} 有空角色名");
        }
        assert!(builtin_names("unknown_game").is_none());
    }

    #[test]
    fn ensure_character_dirs_creates_only_for_mods_path() {
        let base = std::env::temp_dir().join(format!("imm_chars_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        // 路径不含 mods → 不创建任何目录
        let plain = base.join("PlainGame").join("Data");
        let (c, e) = ensure_character_dirs("endfield", &plain.to_string_lossy()).unwrap();
        assert_eq!((c, e), (0, 0));
        assert!(!plain.exists());

        // 路径含 mods → 按固化名单创建
        let mods = base.join("SomeGame").join("Mods");
        let (c, e) = ensure_character_dirs("endfield", &mods.to_string_lossy()).unwrap();
        let expected = builtin_names("endfield").unwrap().len();
        assert_eq!((c, e), (expected, 0));
        assert!(mods.join("伊冯").is_dir());
        assert!(mods.join("UI").is_dir());

        // 再次调用 → 全部已存在，不重复创建
        let (c2, e2) = ensure_character_dirs("endfield", &mods.to_string_lossy()).unwrap();
        assert_eq!((c2, e2), (0, expected));

        // 自定义游戏（无固化名单）→ 不创建
        let mods2 = base.join("CustomGame").join("mods");
        let (c3, e3) = ensure_character_dirs("my_custom_game", &mods2.to_string_lossy()).unwrap();
        assert_eq!((c3, e3), (0, 0));

        let _ = std::fs::remove_dir_all(&base);
    }
}

