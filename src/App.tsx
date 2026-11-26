import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { BUILTIN_COVERS } from "./builtinCovers";
import "./App.css";

type ModEntry = {
  name: string;
  character: string;
  folder_name: string;
  enabled: boolean;
  rel_path: string;
  has_cover: boolean;
  cover: string;
  /** 目录修改时间（Unix 秒） */
  modified: number;
  /** .ini 声明的游戏资源 hash（冲突检测用） */
  hashes: string[];
};

type CharacterGroup = {
  name: string;
  mods: ModEntry[];
  cover: string;
  enabled_count: number;
  total_count: number;
  /** 角色名全拼（小写，后端生成） */
  pinyin: string;
  /** 角色名拼音首字母（后端生成） */
  pinyin_initials: string;
};

type Preset = {
  name: string;
  enabled_mods: string[];
  created_at: string;
};

/** 角色皮肤分类（外观分区） */
type CharacterSection = {
  id: string;
  character: string;
  name: string;
  created_at: string;
};

/** QAQM 内置的官方皮肤定义 */
type OfficialSkin = {
  sectionId: string;
  nameZh: string;
  nameEn: string;
  image: string;
};

/** Mod 快捷键（解析自 mod.ini 的 [Key*] 段） */
type ModHotkey = {
  section: string;
  description: string;
  keys: string[];
  raw: string;
};

/** 拖拽导入候选 */
type ImportCandidate = {
  path: string;
  name: string;
  character: string;
  size: number;
};

/** 整合包里的单个条目（mod 目录或角色目录下的散文件） */
type PackEntry = {
  name: string;
  kind: string;
  conflict: boolean;
  disabled: boolean;
};

type PackCharacter = {
  name: string;
  exists: boolean;
  entries: PackEntry[];
  mod_count: number;
};

/** 一个可能的「Mods 层」 */
type PackCandidate = {
  path: string;
  depth: number;
  named_mods: boolean;
  character_count: number;
  mod_count: number;
  matched: number;
  label: string;
};

type ModpackInfo = {
  source: string;
  mods_root: string;
  temp_dir: string | null;
  mods_path: string;
  candidates: PackCandidate[];
  characters: PackCharacter[];
  character_count: number;
  mod_count: number;
  conflict_count: number;
  loose_file_count: number;
};

type ModpackResult = {
  installed: number;
  skipped: number;
  failed: { name: string; error: string }[];
  characters: number;
  mods: number;
  replaced: number;
  persist_touched: number;
  persist_enabled: boolean;
};

/** 内置游戏信息 */
type BuiltinGameInfo = {
  id: string;
  name: string;
  mods_path: string;
  added: boolean;
  hidden: boolean;
};

type Game = {
  id: string;
  name: string;
  mods_path: string;
  presets: Preset[];
  /** GameBanana 市场绑定（自定义游戏兜底，0 = 未绑定） */
  gb_game_id?: number;
  gb_category_id?: number;
  /** 游戏程序路径（原版启动） */
  game_path?: string;
  /** XXMI Launcher.exe 路径 */
  launcher_path?: string;
  /** GIML.exe 路径（原神加壳启动器） */
  giml_path?: string;
  /** 启动模式（空 = 原版） */
  launch_mode?: string;
  /** 附加启动参数 */
  launch_args?: string;
};

type LaunchConfig = {
  game_path: string;
  launcher_path: string;
  giml_path: string;
  configured_game_path: string;
  configured_launcher_path: string;
  configured_giml_path: string;
  launch_mode: string;
  launch_args: string;
  supported_modes: string[];
  importer: string;
};

type NavKey = "characters" | "market" | "presets" | "games" | "personal" | "settings" | "about";

const NAV_ITEMS: { key: NavKey; label: string; icon: string }[] = [
  { key: "characters", label: "角色一览", icon: "👤" },
  { key: "market", label: "Mod 市场", icon: "🛒" },
  { key: "presets", label: "预设配置", icon: "💾" },
  { key: "games", label: "切换游戏", icon: "🎮" },
  { key: "personal", label: "个性化", icon: "🎨" },
  { key: "settings", label: "系统设置", icon: "⚙️" },
  { key: "about", label: "关于", icon: "ℹ️" },
];

/** 启动按钮里的游戏简称（复刻 QAQM 的 shortName） */
function shortGameName(id: string): string {
  return (
    {
      endfield: "终末地",
      starrail: "星穹铁道",
      wuthering: "鸣潮",
      zenless: "绝区零",
      GenshinImpact: "原神",
    } as Record<string, string>
  )[id] || "游戏";
}

// ===== 主题 =====
type ThemePref = "dark" | "light" | "system";

function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/** 按偏好设置 <html data-theme>；system 档实时解析系统深浅色 */
function applyThemePref(pref: ThemePref) {
  const effective = pref === "system" ? (systemPrefersDark() ? "dark" : "light") : pref;
  document.documentElement.dataset.theme = effective;
}

function App() {
  const [games, setGames] = useState<Game[]>([]);
  const [activeGameId, setActiveGameId] = useState("");
  const [modsPath, setModsPath] = useState("");
  const [groups, setGroups] = useState<CharacterGroup[]>([]);
  const [nav, setNav] = useState<NavKey>("characters");
  const [selectedChar, setSelectedChar] = useState<string>("");
  const [presets, setPresets] = useState<Preset[]>([]);
  const [presetName, setPresetName] = useState("");
  const [error, setError] = useState("");
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  const [persistEnabled, setPersistEnabled] = useState(false);
  const [persistCacheSize, setPersistCacheSize] = useState(30);
  // 角色卡片宽度（系统设置滑块，120~260px，持久化到 config.json）
  const [cardSize, setCardSize] = useState(() => {
    const v = Number(localStorage.getItem("qaqm.cardSize"));
    return v >= 120 && v <= 260 ? v : 158;
  });
  useEffect(() => {
    invoke<number>("get_card_size")
      .then((v) => {
        if (v >= 120 && v <= 260) {
          setCardSize(v);
          localStorage.setItem("qaqm.cardSize", String(v));
        }
      })
      .catch(() => {});
  }, []);
  function handleCardSizeChange(next: number) {
    const v = Math.max(120, Math.min(260, Math.round(next)));
    setCardSize(v);
    localStorage.setItem("qaqm.cardSize", String(v));
    invoke("set_card_size", { size: v }).catch(() => {});
  }

  // 画廊 Mod 卡片宽度（系统设置滑块，140~320px，持久化到 config.json）
  const [galleryCardSize, setGalleryCardSize] = useState(() => {
    const v = Number(localStorage.getItem("qaqm.galleryCardSize"));
    return v >= 140 && v <= 320 ? v : 200;
  });
  useEffect(() => {
    invoke<number>("get_gallery_card_size")
      .then((v) => {
        if (v >= 140 && v <= 320) {
          setGalleryCardSize(v);
          localStorage.setItem("qaqm.galleryCardSize", String(v));
        }
      })
      .catch(() => {});
  }, []);
  function handleGalleryCardSizeChange(next: number) {
    const v = Math.max(140, Math.min(320, Math.round(next)));
    setGalleryCardSize(v);
    localStorage.setItem("qaqm.galleryCardSize", String(v));
    invoke("set_gallery_card_size", { size: v }).catch(() => {});
  }

  // Mod 市场中央推荐大卡片大小（系统设置滑块，40~100%，等比缩放：宽度变，高度按比例跟随）
  const [marketHeroScale, setMarketHeroScale] = useState(() => {
    const v = Number(localStorage.getItem("qaqm.marketHeroScale"));
    return v >= 40 && v <= 100 ? v : 100;
  });
  useEffect(() => {
    invoke<number>("get_market_hero_scale")
      .then((v) => {
        if (v >= 40 && v <= 100) {
          setMarketHeroScale(v);
          localStorage.setItem("qaqm.marketHeroScale", String(v));
        }
      })
      .catch(() => {});
  }, []);
  function handleMarketHeroScaleChange(next: number) {
    const v = Math.max(40, Math.min(100, Math.round(next)));
    setMarketHeroScale(v);
    localStorage.setItem("qaqm.marketHeroScale", String(v));
    invoke("set_market_hero_scale", { scale: v }).catch(() => {});
  }

  // Mod 市场小卡片宽度（系统设置滑块，150~320px）
  const [marketCardSize, setMarketCardSize] = useState(() => {
    const v = Number(localStorage.getItem("qaqm.marketCardSize"));
    return v >= 150 && v <= 320 ? v : 210;
  });
  useEffect(() => {
    invoke<number>("get_market_card_size")
      .then((v) => {
        if (v >= 150 && v <= 320) {
          setMarketCardSize(v);
          localStorage.setItem("qaqm.marketCardSize", String(v));
        }
      })
      .catch(() => {});
  }, []);
  function handleMarketCardSizeChange(next: number) {
    const v = Math.max(150, Math.min(320, Math.round(next)));
    setMarketCardSize(v);
    localStorage.setItem("qaqm.marketCardSize", String(v));
    invoke("set_market_card_size", { size: v }).catch(() => {});
  }
  const [skinsCatalog, setSkinsCatalog] = useState<Record<string, { characters: any[] }>>({});
  // 市场「仅下载」保存文件夹（空 = 系统默认下载文件夹）
  const [downloadDir, setDownloadDir] = useState("");
  // 全局 XXMI Launcher 路径（所有游戏共用，一次设置）
  const [xxmiPath, setXxmiPath] = useState("");
  // 启动游戏（复刻 QAQM）：当前游戏的启动配置与临时选中的模式
  const [launchConfig, setLaunchConfig] = useState<LaunchConfig | null>(null);
  const [launchMode, setLaunchMode] = useState<string>("");
  const [launching, setLaunching] = useState(false);
  // 全局下载中心：进行中的下载任务（事件驱动，不持久化，退出程序即消失）
  const [dlTasks, setDlTasks] = useState<DlTask[]>([]);
  // 下载中心默认收起（右下角图标）；新任务到达或用户点击时展开
  const [dlCenterMin, setDlCenterMin] = useState(true);
  // 更新检测结果：有新版本的市场安装 Mod（key = rel_path）
  const [updateKeys, setUpdateKeys] = useState<string[]>([]);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const updatesCheckedRef = useRef(false);
  async function checkModUpdates(force = false) {
    if (checkingUpdates) return;
    if (!force && updatesCheckedRef.current) return; // 本次会话只自动检测一次
    setCheckingUpdates(true);
    try {
      const list = await invoke<
        { rel_path: string; gb_mod_id: number; page_url: string; latest_ts: number; installed_at: number }[]
      >("market_check_updates");
      setUpdateKeys(list.map((x) => x.rel_path));
      updatesCheckedRef.current = true;
      if (list.length > 0) {
        setMsg(`检测到 ${list.length} 个 Mod 有新版本`);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setCheckingUpdates(false);
    }
  }
  useEffect(() => {
    let un: (() => void) | undefined;
    listen<{ taskId?: string; filename: string; downloaded: number; total: number | null; cover_error?: string }>(
      "market-dl-progress",
      (ev) => {
        const p = ev.payload;
        const taskId = p.taskId;
        if (p.cover_error || !taskId) return; // 封面事件 / 无任务 id 不入列表
        setDlTasks((prev) => {
          const i = prev.findIndex((x) => x.id === taskId);
          if (i === -1) {
            // 新任务：主动展开下载中心，合并触发时暂存的页面链接
            setDlCenterMin(false);
            const modUrl = pendingTaskUrls.current.get(taskId);
            return [...prev, { id: taskId, filename: p.filename, downloaded: p.downloaded, total: p.total, status: "active", modUrl }];
          }
          const next = [...prev];
          // 终态任务不再被进度事件覆盖
          if (next[i].status !== "active") return prev;
          next[i] = { ...next[i], filename: p.filename, downloaded: p.downloaded, total: p.total };
          return next;
        });
      }
    ).then((fn) => {
      un = fn;
    });
    return () => {
      if (un) un();
    };
  }, []);
  function cancelDlTask(taskId: string) {
    invoke("cancel_market_download", { taskId }).catch((e) => {
      console.warn("[dl] cancel failed", taskId, String(e));
    });
  }
  function removeDlTask(taskId: string) {
    setDlTasks((prev) => prev.filter((t) => t.id !== taskId));
  }
  // 触发下载时先暂存 GameBanana 页面链接（任务注册事件稍后才到，届时合并）
  const pendingTaskUrls = useRef(new Map<string, string>());
  /** 触发下载时登记任务对应的 GameBanana 页面链接 */
  function registerTaskUrl(taskId: string, modUrl: string) {
    pendingTaskUrls.current.set(taskId, modUrl);
    // 任务可能已注册（事件先到的情况），就地合并
    setDlTasks((prev) => prev.map((t) => (t.id === taskId ? { ...t, modUrl } : t)));
  }
  useEffect(() => {
    invoke<string>("get_download_dir")
      .then((d) => setDownloadDir(d || ""))
      .catch(() => {});
    invoke<string>("get_xxmi_launcher_path")
      .then((p) => setXxmiPath(p || ""))
      .catch(() => {});
  }, []);
  async function handleChangeDownloadDir() {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked !== "string") return;
      await invoke("set_download_dir", { path: picked });
      setDownloadDir(picked);
      setMsg("下载文件夹已更新");
    } catch (e) {
      setError(String(e));
    }
  }
  /** 更改全局 XXMI Launcher 路径（所有游戏共用） */
  async function handleChangeXxmiPath() {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "程序", extensions: ["exe"] }],
      });
      if (typeof picked !== "string") return;
      await invoke("set_xxmi_launcher_path", { path: picked });
      setXxmiPath(picked);
      setMsg("XXMI 启动器路径已更新（对所有游戏生效）");
    } catch (e) {
      setError(String(e));
    }
  }

  /** 更改「当前激活游戏」的 Mods 路径（系统设置 → 当前游戏） */
  async function handleChangeModsPath() {
    const game = games.find((g) => g.id === activeGameId);
    if (!game) {
      setError("尚未选择游戏");
      return;
    }
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked !== "string") return;
      await invoke("set_game_mods_path", {
        id: game.id,
        name: game.name,
        modsPath: picked,
      });
      setMsg("Mods 路径已更新");
      await refreshGames();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  /** 切换启动模式：再次点击同一模式则取消（回到原版），复刻 QAQM；选择结果按游戏记住 */
  async function toggleLaunchMode(mode: string) {
    const next = launchMode === mode ? "" : mode;
    setLaunchMode(next);
    if (!activeGameId) return;
    const cfg = launchConfig;
    const g = games.find((x) => x.id === activeGameId);
    if (!cfg && !g) return;
    try {
      // 只改 launch_mode，路径/参数沿用已有值，避免覆盖用户在设置页配置的路径
      await invoke("set_launch_config", {
        id: activeGameId,
        gamePath: cfg?.configured_game_path ?? g?.game_path ?? "",
        launcherPath: cfg?.configured_launcher_path ?? g?.launcher_path ?? "",
        gimlPath: cfg?.configured_giml_path ?? g?.giml_path ?? "",
        launchMode: next,
        launchArgs: cfg?.launch_args ?? g?.launch_args ?? "",
      });
      setLaunchConfig((prev) => (prev ? { ...prev, launch_mode: next } : prev));
      setGames((prev) =>
        prev.map((x) => (x.id === activeGameId ? { ...x, launch_mode: next } : x))
      );
    } catch (e) {
      setError(`保存启动模式失败：${String(e)}`);
    }
  }

  /** 启动游戏（复刻 QAQM 的 launch-game） */
  async function handleLaunch() {
    if (launching) return;
    if (!activeGameId) {
      setError("尚未选择游戏");
      return;
    }
    setLaunching(true);
    setError("");
    try {
      await invoke("launch_game", { id: activeGameId, launchMode });
      setMsg(launchMode ? `正在启动（${launchMode}）` : "正在原版启动游戏");
    } catch (e) {
      setError(`启动失败：${String(e)}`);
    } finally {
      setLaunching(false);
    }
  }

  /** 保存启动路径/模式/参数（系统设置） */
  async function handleSaveLaunchConfig(patch: {
    game_path?: string;
    launcher_path?: string;
    giml_path?: string;
    launch_mode?: string;
    launch_args?: string;
  }) {
    try {
      const cur = launchConfig;
      await invoke("set_launch_config", {
        id: activeGameId,
        gamePath: patch.game_path ?? cur?.configured_game_path ?? "",
        launcherPath: patch.launcher_path ?? cur?.configured_launcher_path ?? "",
        gimlPath: patch.giml_path ?? cur?.configured_giml_path ?? "",
        launchMode: patch.launch_mode ?? cur?.launch_mode ?? "",
        launchArgs: patch.launch_args ?? cur?.launch_args ?? "",
      });
      await refreshLaunchConfig();
      setMsg("启动设置已保存");
    } catch (e) {
      setError(String(e));
    }
  }

  const [dragActive, setDragActive] = useState(false);
  // 退出确认弹窗（点窗口关闭按钮时由后端事件触发）
  const [confirmExit, setConfirmExit] = useState(false);
  // 主题：启动时恢复偏好（深色/浅色/跟随系统），存 localStorage
  useEffect(() => {
    let saved = localStorage.getItem("emm-theme-pref") as ThemePref | null;
    if (!saved) {
      // 兼容旧版只存深/浅的键
      const legacy = localStorage.getItem("emm-theme");
      saved = legacy === "light" || legacy === "dark" ? legacy : "system";
    }
    if (saved !== "dark" && saved !== "light" && saved !== "system") saved = "system";
    localStorage.setItem("emm-theme-pref", saved);
    localStorage.removeItem("emm-theme");
    applyThemePref(saved);
    // 跟随系统档：系统切换深浅色时实时跟随
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      const cur = (localStorage.getItem("emm-theme-pref") as ThemePref) || "system";
      if (cur === "system") applyThemePref("system");
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  // 启动时自动检查更新：发现新版则弹窗提示；失败时用底部提示窗告知（不弹窗打扰）
  const [startupUpdate, setStartupUpdate] = useState<AppUpdateInfo | null>(null);
  useEffect(() => {
    const t = window.setTimeout(() => {
      invoke<AppUpdateInfo>("check_app_update")
        .then((u) => {
          if (u?.hasUpdate) setStartupUpdate(u);
        })
        .catch((e) => {
          setError(`检查更新失败（已跳过）：${String(e)}`);
        });
    }, 1200);
    return () => window.clearTimeout(t);
  }, []);
  const [importCandidates, setImportCandidates] = useState<ImportCandidate[] | null>(null);
  // 整合包导入弹窗
  const [packOpen, setPackOpen] = useState(false);

  async function refreshGames() {
    const gs = await invoke<Game[]>("list_games");
    setGames(gs);
    const active = await invoke<string>("get_active_game_id");
    setActiveGameId(active);
  }

  /** 加载当前游戏的启动配置（路径 + 支持的模式 + 当前模式） */
  async function refreshLaunchConfig() {
    if (!activeGameId) {
      setLaunchConfig(null);
      setLaunchMode("");
      return;
    }
    try {
      const cfg = await invoke<LaunchConfig>("get_launch_config", { id: activeGameId });
      setLaunchConfig(cfg);
      setLaunchMode(cfg.launch_mode || "");
    } catch {
      setLaunchConfig(null);
      setLaunchMode("");
    }
  }

  async function refresh() {
    try {
      const path = await invoke<string>("get_mods_path");
      setModsPath(path);
      if (path) {
        const gs = await invoke<CharacterGroup[]>("scan");
        setGroups(gs);
        // 启动停留在角色一览，不自动进入任何角色
      } else {
        setGroups([]);
        setSelectedChar("");
      }
      const ps = await invoke<Preset[]>("list_presets");
      setPresets(ps);
      const persist = await invoke<{ enabled: boolean; cache_size: number }>("get_persist_status");
      setPersistEnabled(persist.enabled);
      setPersistCacheSize(persist.cache_size);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    (async () => {
      try {
        await refreshGames();
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  // 切换游戏后刷新启动配置（activeGameId 变更驱动）
  useEffect(() => {
    refreshLaunchConfig();
  }, [activeGameId]);

  // 加载内置皮肤目录（QAQM 迁移数据）
  useEffect(() => {
    fetch("/qaqm-skins/skins-catalog.json")
      .then((r) => (r.ok ? r.json() : {}))
      .then((d) => setSkinsCatalog(d || {}))
      .catch(() => setSkinsCatalog({}));
  }, []);

  // 点窗口关闭按钮：后端拦下 CloseRequested 并发事件，由前端二次确认
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen("request-exit", () => setConfirmExit(true))
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // 提权自检：以管理员权限运行时，Windows(UIPI) 会静默阻止把文件拖进窗口
  useEffect(() => {
    invoke<boolean>("is_elevated")
      .then((elevated) => {
        if (elevated) {
          setMsg(
            "当前以管理员权限运行，Windows 会阻止把文件拖进窗口。拖拽导入请以普通权限重新打开程序，或改用「批量导入」按钮。",
          );
        }
      })
      .catch(() => {});
  }, []);

  // 拖拽导入：监听窗口级拖放事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const win = getCurrentWindow();
        const un = await win.onDragDropEvent((event) => {
          const p = event.payload;
          if (p.type === "enter" || p.type === "over") {
            setDragActive(true);
          } else if (p.type === "drop") {
            setDragActive(false);
            const paths = p.paths || [];
            if (paths.length > 0) {
              invoke<ImportCandidate[]>("analyze_imports", { paths })
                .then((list) => {
                  if (list.length === 0) {
                    setError("未识别到可导入的内容（支持 zip / rar / 7z 压缩包，或 mod 文件夹）");
                  } else {
                    setImportCandidates(list);
                  }
                })
                .catch((e) => setError(String(e)));
            }
          } else {
            setDragActive(false);
          }
        });
        if (cancelled) un();
        else unlisten = un;
      } catch {
        /* 忽略 */
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // 窗口尺寸：启动时恢复上次大小，否则用默认 1440x900 并居中
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const win = getCurrentWindow();
        const saved = localStorage.getItem("qaqm.window.size");
        if (saved) {
          const { width, height } = JSON.parse(saved);
          if (width > 0 && height > 0) {
            await win.setSize(new LogicalSize(width, height));
          }
        } else {
          await win.setSize(new LogicalSize(1440, 900));
          await win.center();
        }
        // 记住用户调整后的尺寸（防抖保存）
        let timer: number | undefined;
        unlisten = await win.onResized(({ payload }) => {
          if (timer) window.clearTimeout(timer);
          timer = window.setTimeout(async () => {
            try {
              const scale = await win.scaleFactor();
              const w = Math.round(payload.width / scale);
              const h = Math.round(payload.height / scale);
              if (w > 400 && h > 300) {
                localStorage.setItem("qaqm.window.size", JSON.stringify({ width: w, height: h }));
              }
            } catch {
              /* ignore */
            }
          }, 600);
        });
      } catch {
        /* 非 Tauri 环境忽略 */
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  async function handleSwitchGame(id: string) {
    setError("");
    try {
      await invoke("set_active_game", { id });
      setSelectedChar("");
      // 更新检测结果随游戏重置（每游戏独立的安装来源记录）
      setUpdateKeys([]);
      updatesCheckedRef.current = false;
      // 先清空旧游戏数据，避免刷新间隙闪现上一游戏角色（配合 CharactersView 的 key 强制重挂载）
      setGroups([]);
      await refreshGames();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleAddGame() {
    setError("");
    setMsg("");
    try {
      const picked = await open({ directory: true });
      if (typeof picked === "string") {
        await invoke("add_game", { modsPath: picked });
        setMsg("游戏已添加");
        await refreshGames();
        await refresh();
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRemoveGame(id: string) {
    if (!id) return;
    setError("");
    try {
      await invoke("remove_game", { id });
      setMsg("游戏已移除");
      await refreshGames();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleToggle(m: ModEntry) {
    setError("");
    try {
      await invoke("toggle_mod", {
        character: m.character,
        modName: m.name,
        enabled: !m.enabled,
      });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleImport() {
    setError("");
    setMsg("");
    const char = selectedChar || groups[0]?.name;
    if (!char) return;
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "压缩包", extensions: ["zip", "rar", "7z"] }],
      });
      if (typeof picked === "string") {
        setBusy(true);
        const name = await invoke<string>("import", { archive: picked, character: char });
        setMsg(`已导入: ${name}`);
        await refresh();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSavePreset() {
    setError("");
    setMsg("");
    if (!presetName.trim()) {
      setError("请输入预设名称");
      return;
    }
    try {
      await invoke("save_preset", { name: presetName.trim() });
      setMsg(`预设「${presetName.trim()}」已保存`);
      setPresetName("");
      const ps = await invoke<Preset[]>("list_presets");
      setPresets(ps);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleApplyPreset(name: string) {
    setError("");
    try {
      await invoke("apply_preset", { name });
      setMsg(`已应用预设「${name}」`);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleDeletePreset(name: string) {
    try {
      await invoke("delete_preset", { name });
      const ps = await invoke<Preset[]>("list_presets");
      setPresets(ps);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleTogglePersist() {
    setError("");
    setMsg("");
    try {
      const next = !persistEnabled;
      await invoke("set_persist_enabled", { enabled: next });
      setPersistEnabled(next);
      setMsg(next ? "Persist Bridge 已启用（需重启游戏生效）" : "Persist Bridge 已禁用，已还原 mod 文件");
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleClearPersistCache() {
    setError("");
    try {
      const n = await invoke<number>("clear_persist_cache");
      setMsg(`已清空 ${n} 个 persist 缓存文件`);
    } catch (e) {
      setError(String(e));
    }
  }

  /** 手动选择压缩包批量导入（与拖拽走同一流程） */
  async function handlePickArchives() {
    setError("");
    try {
      const picked = await open({
        multiple: true,
        filters: [{ name: "压缩包", extensions: ["zip", "rar", "7z"] }],
      });
      const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      if (paths.length === 0) return;
      const list = await invoke<ImportCandidate[]>("analyze_imports", { paths });
      if (list.length === 0) {
        setError("未识别到可导入的内容（支持 zip / rar / 7z 压缩包，或 mod 文件夹）");
      } else {
        setImportCandidates(list);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleBatchImport(
    items: { path: string; character: string; name: string }[]
  ) {
    setError("");
    setMsg("");
    setBusy(true);
    try {
      const done = await invoke<string[]>("batch_import", { items });
      setMsg(`已成功导入 ${done.length} 个 Mod`);
      setImportCandidates(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /** 整合包导入完成 */
  function handlePackDone(r: ModpackResult) {
    setPackOpen(false);
    const parts = [`整合包导入完成：${r.characters} 个角色 / ${r.mods} 个 Mod`];
    if (r.replaced) parts.push(`覆盖 ${r.replaced} 个同名`);
    if (r.skipped) parts.push(`跳过 ${r.skipped} 个同名`);
    if (r.failed.length) parts.push(`失败 ${r.failed.length} 个`);
    if (r.installed > 0) {
      setError("");
      setMsg(parts.join("，"));
    } else if (r.skipped > 0) {
      setError("");
      setMsg(`全部 ${r.skipped} 项同名已跳过（需要覆盖请勾选「覆盖同名」）`);
    } else {
      setMsg("");
      setError(
        r.failed.length
          ? `整合包导入失败：${r.failed[0].name}（${r.failed[0].error}）`
          : "整合包没有导入任何内容"
      );
    }
    refresh();
  }

  async function handleAddCharacter(name: string) {
    setError("");
    setMsg("");
    try {
      const created = await invoke<string>("add_character", { name });
      setMsg(`已创建角色「${created}」`);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRenameCharacter(oldName: string, newName: string) {
    setError("");
    try {
      const applied = await invoke<string>("rename_character", {
        oldName,
        newName,
      });
      setMsg(`角色已重命名为「${applied}」`);
      if (selectedChar === oldName) setSelectedChar(applied);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCacheSizeChange(size: number) {
    setError("");
    try {
      const applied = await invoke<number>("set_persist_cache_size", { size });
      setPersistCacheSize(applied);
    } catch (e) {
      setError(String(e));
    }
  }

  /** 退出弹窗「取消」：仅关闭弹窗，程序继续运行 */
  function handleExitCancel() {
    setConfirmExit(false);
  }

  /** 只有点「退出」才真正退出进程 */
  function handleExitConfirm() {
    invoke("exit_app").catch(() => {});
  }

  return (
    <div className="app">
      <header className="topbar">
        <div
          className="brand"
          role="link"
          tabIndex={0}
          title="跳转作者主页"
          onClick={() => openUrl("https://github.com/KobayashiWakaba1211").catch(() => {})}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              openUrl("https://github.com/KobayashiWakaba1211").catch(() => {});
            }
          }}
        >
          小林若葉Manager
        </div>
        <span className="current-game">
          {games.find((g) => g.id === activeGameId)?.name || "未选择游戏"}
        </span>
        <span
          className={`path-display ${modsPath ? "" : "path-missing"}`}
          title={modsPath ? modsPath : "点击配置当前游戏的 Mods 目录"}
          onClick={() => {
            if (!modsPath) {
              setNav("settings");
            }
          }}
        >
          {modsPath || "未配置 Mods 目录（点击设置）"}
        </span>
      </header>

      {/* 底部居中浮层提示（自动关闭，悬停暂停） */}
      {(error || msg) && (
        <Toast
          key={`${error}|${msg}`}
          type={error ? "error" : "ok"}
          text={error || msg}
          onClose={() => {
            setError("");
            setMsg("");
          }}
        />
      )}

      <div className="layout">
        {/* 左侧导航 */}
        <nav className="sidebar">
          <ul className="nav-list">
            {NAV_ITEMS.map((item) => (
              <li
                key={item.key}
                className={nav === item.key ? "active" : ""}
                onClick={() => {
                  setNav(item.key);
                  // 切到「角色一览」时回到角色网格
                  if (item.key === "characters") setSelectedChar("");
                }}
              >
                <span className="nav-icon">{item.icon}</span>
                <span className="nav-label">{item.label}</span>
              </li>
            ))}
          </ul>

          {/* 启动面板（复刻 QAQM） */}
          <div className="launch-panel">
            <div className="launch-mode-label">启动模式</div>
            <div className="launch-mode-buttons">
              {(launchConfig?.supported_modes ?? []).map((mode) => (
                <button
                  key={mode}
                  className={`launch-mode-btn ${launchMode === mode ? "active" : ""}`}
                  onClick={() => toggleLaunchMode(mode)}
                  title={
                    mode === "XXMI"
                      ? "带界面启动 XXMI Launcher"
                      : mode === "GIML"
                        ? "通过 GIML 加壳启动器启动（帧数解锁）"
                        : `通过 XXMI 注入 ${mode} 启动`
                  }
                >
                  {mode}
                </button>
              ))}
            </div>
            <button
              className="launch-primary-btn"
              onClick={handleLaunch}
              disabled={launching}
            >
              {launching ? (
                <><span className="launch-spinner" /> 启动中...</>
              ) : (
                <>{launchMode ? `🚀 启动 ${shortGameName(activeGameId)} (${launchMode})` : `🚀 原版启动 ${shortGameName(activeGameId)}`}</>
              )}
            </button>
          </div>
        </nav>

        {/* 右侧内容 */}
        <main className="content">
          {games.length === 0 ? (
            <div className="empty">
              <div>
                <p>尚未添加任何游戏</p>
                <p className="hint">点击顶部「＋」，选择游戏的 Mods 目录（如 XXMI MODS\EndField Game\Mods）</p>
                <button className="primary" onClick={handleAddGame}>＋ 添加游戏</button>
              </div>
            </div>
          ) : nav === "characters" ? (
            !modsPath ? (
              <div className="empty">
                <div>
                  <p>尚未配置 Mods 目录</p>
                  <p className="hint">
                    当前游戏「{games.find((g) => g.id === activeGameId)?.name || "未选择"}」还没有设置 Mods
                    文件夹，请先选择它的 Mods 目录（例如 XXMI MODS\EndField Game\Mods）。
                  </p>
                  <button className="primary" onClick={() => setNav("settings")}>
                    ⚙️ 去系统设置配置
                  </button>
                </div>
              </div>
            ) : selectedChar && groups.some((g) => g.name === selectedChar) ? (
              <CharacterDetailView
                group={groups.find((g) => g.name === selectedChar)!}
                allCharacterNames={groups.map((g) => g.name)}
                gameId={activeGameId}
                skinsCatalog={skinsCatalog}
                modsPath={modsPath}
                onToggle={handleToggle}
                onImport={handleImport}
                onBack={() => setSelectedChar("")}
                busy={busy}
                onRefresh={refresh}
                galleryCardSize={galleryCardSize}
                setMsg={setMsg}
                setError={setError}
              />
            ) : (
              <CharactersView
                key={activeGameId}
                groups={groups}
                gameId={activeGameId}
                skinsCatalog={skinsCatalog}
                modsPath={modsPath}
                selectedChar={selectedChar}
                onSelect={setSelectedChar}
                onRename={handleRenameCharacter}
                onAddCharacter={handleAddCharacter}
                onPickArchives={handlePickArchives}
                onOpenPack={() => setPackOpen(true)}
                onRefreshAll={refresh}
                cardSize={cardSize}
                updateKeys={updateKeys}
                checkingUpdates={checkingUpdates}
                onCheckUpdates={checkModUpdates}
              />
            )
          ) : nav === "market" ? (
            <MarketView
              key={activeGameId}
              gameId={activeGameId}
              groups={groups}
              modsPath={modsPath}
              initialChar={selectedChar}
              heroScale={marketHeroScale}
              cardSize={marketCardSize}
              onInstalled={refresh}
              setMsg={setMsg}
              setError={setError}
              dlTasks={dlTasks}
              onTaskEnd={(id, status, note, targetPath) => {
                pendingTaskUrls.current.delete(id);
                setDlTasks((prev) =>
                  prev.map((t) => (t.id === id ? { ...t, status, note, targetPath } : t)),
                );
              }}
              onRegisterUrl={registerTaskUrl}
              onCancelTask={cancelDlTask}
            />
          ) : nav === "presets" ? (
            <PresetsView
              presets={presets}
              presetName={presetName}
              setPresetName={setPresetName}
              onSave={handleSavePreset}
              onApply={handleApplyPreset}
              onDelete={handleDeletePreset}
            />
          ) : nav === "games" ? (
            <GamesView
              games={games}
              activeGameId={activeGameId}
              onSwitch={handleSwitchGame}
              onAddGame={handleAddGame}
              onRemoveGame={handleRemoveGame}
              onRefreshGames={refreshGames}
            />
          ) : nav === "personal" ? (
            <PersonalizationView
              cardSize={cardSize}
              onCardSizeChange={handleCardSizeChange}
              galleryCardSize={galleryCardSize}
              onGalleryCardSizeChange={handleGalleryCardSizeChange}
              marketHeroScale={marketHeroScale}
              onMarketHeroScaleChange={handleMarketHeroScaleChange}
              marketCardSize={marketCardSize}
              onMarketCardSizeChange={handleMarketCardSizeChange}
            />
          ) : nav === "about" ? (
            <AboutView />
          ) : (
            <SettingsView
              persistEnabled={persistEnabled}
              persistCacheSize={persistCacheSize}
              onTogglePersist={handleTogglePersist}
              onClearPersist={handleClearPersistCache}
              onCacheSizeChange={handleCacheSizeChange}
              downloadDir={downloadDir}
              onChangeDownloadDir={handleChangeDownloadDir}
              activeGameName={games.find((g) => g.id === activeGameId)?.name || activeGameId}
              activeModsPath={modsPath}
              onChangeModsPath={handleChangeModsPath}
              xxmiPath={xxmiPath}
              onChangeXxmiPath={handleChangeXxmiPath}
              launchConfig={launchConfig}
              onSaveLaunchConfig={handleSaveLaunchConfig}
            />
          )}
        </main>
      </div>

      {/* 全局下载中心（右下角） */}
      <DlCenter
        tasks={dlTasks}
        minimized={dlCenterMin}
        onToggle={() => setDlCenterMin(!dlCenterMin)}
        onCancel={cancelDlTask}
        onRemove={removeDlTask}
      />

      {/* 退出确认（二级菜单） */}
      {confirmExit && (
        <div className="modal-overlay" onClick={() => setConfirmExit(false)}>
          <div className="modal exit-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>退出程序</h3>
            </div>
            <div className="exit-body">
              <p>确定要退出「小林若葉Manager」吗？</p>
            </div>
            <div className="modal-foot exit-foot">
              <div className="modal-actions">
                <button onClick={handleExitCancel}>取消</button>
                <button className="primary" onClick={handleExitConfirm}>
                  退出
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 启动时发现新版本的弹窗 */}
      {startupUpdate && (
        <div className="modal-overlay" onClick={() => setStartupUpdate(null)}>
          <div className="modal update-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>🚀 发现新版本 v{startupUpdate.latestVersion}</h3>
            </div>
            <div className="exit-body">
              <p>
                当前版本 v{startupUpdate.currentVersion}
                {startupUpdate.publishedAt && ` · 新版发布于 ${startupUpdate.publishedAt}`}
                ，建议更新以获得修复与新功能。
              </p>
              {startupUpdate.notes && (
                <pre className="update-modal-notes">{startupUpdate.notes}</pre>
              )}
            </div>
            <div className="modal-foot exit-foot">
              <div className="modal-actions">
                <button onClick={() => setStartupUpdate(null)}>稍后再说</button>
                <button
                  onClick={() => {
                    openUrl(startupUpdate.releaseUrl).catch(() => {});
                  }}
                >
                  打开发布页
                </button>
                <button
                  className="primary"
                  onClick={() => {
                    openUrl(startupUpdate.downloadUrl).catch(() => {});
                  }}
                  title={startupUpdate.downloadUrl}
                >
                  ⬇ 下载安装包
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 拖拽遮罩 */}
      {dragActive && (
        <div className="drag-overlay">
          <div className="drag-overlay-inner">
            <div className="drag-icon">📥</div>
            <div className="drag-text">拖入压缩包 / Mod 文件夹 / 修复器</div>
            <div className="drag-sub">松开鼠标开始识别并导入</div>
          </div>
        </div>
      )}

      {/* 批量导入弹窗 */}
      {importCandidates && (
        <ImportDialog
          items={importCandidates}
          characters={groups.map((g) => g.name)}
          busy={busy}
          onCancel={() => setImportCandidates(null)}
          onConfirm={handleBatchImport}
        />
      )}

      {/* 整合包导入弹窗 */}
      {packOpen && (
        <ModpackDialog onClose={() => setPackOpen(false)} onDone={handlePackDone} />
      )}
    </div>
  );
}

// ===== 底部浮层提示（Toast） =====
function Toast({
  type,
  text,
  onClose,
  duration = 5000,
}: {
  type: "error" | "ok";
  text: string;
  onClose: () => void;
  duration?: number;
}) {
  const [paused, setPaused] = useState(false);
  const [resumeKey, setResumeKey] = useState(0);
  // 用 ref 保存回调，避免父组件重渲染导致计时被重置
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (paused) return;
    const t = window.setTimeout(() => onCloseRef.current(), duration);
    return () => window.clearTimeout(t);
  }, [paused, duration, resumeKey]);

  return (
    <div
      className={`toast ${type}`}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => {
        setPaused(false);
        setResumeKey((k) => k + 1);
      }}
    >
      <span className="toast-icon">{type === "error" ? "⚠️" : "✓"}</span>
      <span className="toast-text">{text}</span>
      <button className="toast-close" title="关闭" onClick={onClose}>
        ✕
      </button>
      <span
        key={resumeKey}
        className={`toast-progress ${paused ? "paused" : ""}`}
        style={{ animationDuration: `${duration}ms` }}
      />
    </div>
  );
}

// ===== 角色选择器（单击展开 + 搜索 + 可新建） =====
function CharacterPicker({
  value,
  characters,
  onChange,
}: {
  value: string;
  characters: string[];
  onChange: (name: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [popStyle, setPopStyle] = useState<React.CSSProperties>({});
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // 展开后把内联搜索框聚焦（此时它已替换按钮，处于正常文档流）
  useEffect(() => {
    if (!open) return;
    const t = window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => window.clearTimeout(t);
  }, [open]);

  // 弹层用 fixed + 计算坐标：避免被 .import-list / .modal 的 overflow 裁掉
  function openMenu() {
    const el = btnRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const width = Math.max(r.width, 220);
    const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
    const spaceBelow = window.innerHeight - r.bottom - 12;
    const openUp = spaceBelow < 200 && r.top > spaceBelow;
    setPopStyle(
      openUp
        ? { left, bottom: window.innerHeight - r.top + 4, width }
        : { left, top: r.bottom + 4, width },
    );
    setQuery("");
    setOpen(true);
  }

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (
        popRef.current?.contains(t) ||
        btnRef.current?.contains(t) ||
        inputRef.current?.contains(t)
      ) {
        return;
      }
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    // 只在外层滚动时关闭；列表自身滚动 / 输入时内容收缩引起的滚动不能关
    const onScroll = (e: Event) => {
      const t = e.target;
      if (t instanceof Node && popRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onResize = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    document.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  const q = query.trim();
  const lower = q.toLowerCase();
  const filtered = q ? characters.filter((c) => c.toLowerCase().includes(lower)) : characters;
  const exact = characters.some((c) => c === q);
  const invalid = /[\\/:*?"<>|]/.test(q);
  const canCreate = q.length > 0 && !exact && !invalid;

  function pick(name: string) {
    onChange(name);
    setOpen(false);
  }

  function onSearchKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (canCreate) pick(q);
    else if (filtered.length > 0) pick(filtered[0]);
  }

  return (
    <>
      {open ? (
        <input
          ref={inputRef}
          className="char-picker-input"
          value={query}
          placeholder="搜索或输入新角色名…"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onSearchKeyDown}
          autoComplete="off"
          spellCheck={false}
        />
      ) : (
        <button
          ref={btnRef}
          type="button"
          className={`char-picker-btn ${value ? "matched" : "unmatched"}`}
          title={value || "点击选择，或输入新角色名"}
          onClick={openMenu}
        >
          <span className="char-picker-text">{value || "选择角色…"}</span>
          <span className="char-picker-caret">▾</span>
        </button>
      )}

      {open &&
        createPortal(
          <div
            ref={popRef}
            className="char-picker-pop"
            style={popStyle}
            onMouseDown={(e) => e.stopPropagation()}
          >
            <div className="char-picker-list">
              {filtered.map((c) => (
                <button
                  key={c}
                  type="button"
                  className={`char-picker-item ${c === value ? "active" : ""}`}
                  onClick={() => pick(c)}
                >
                  {c}
                </button>
              ))}
              {canCreate && (
                <button
                  type="button"
                  className="char-picker-item create"
                  onClick={() => pick(q)}
                  title="安装时会自动创建该角色文件夹"
                >
                  ＋ 新建角色「{q}」
                </button>
              )}
              {filtered.length === 0 && !canCreate && (
                <div className="char-picker-empty">
                  {invalid ? '角色名不能包含 \\ / : * ? " < > |' : "没有匹配的角色"}
                </div>
              )}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

// ===== 批量导入弹窗 =====
function ImportDialog({
  items,
  characters,
  busy,
  onCancel,
  onConfirm,
}: {
  items: ImportCandidate[];
  characters: string[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: (list: { path: string; character: string; name: string }[]) => void;
}) {
  const [rows, setRows] = useState(
    items.map((it) => ({
      ...it,
      selected: true,
      character: it.character || "",
      name: it.name,
    }))
  );

  // 本次弹窗内可选的角色列表：包含已有角色 + 本轮新建的（新建后其他行也能直接选）
  const [chars, setChars] = useState<string[]>(characters);

  function pickCharacter(index: number, name: string) {
    setRows((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], character: name };
      return next;
    });
    setChars((prev) => (prev.includes(name) ? prev : [...prev, name]));
  }

  const selectedCount = rows.filter((r) => r.selected && r.character).length;
  const matchedCount = rows.filter((r) => r.character).length;

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>📦 批量导入 Mod</h3>
          <button className="modal-close" onClick={onCancel}>
            ✕
          </button>
        </div>

        <div className="modal-tip">
          已识别 <strong>{rows.length}</strong> 项，
          <span className="ok-text">{matchedCount} 项自动匹配角色</span>。
          绿色为已匹配，橙色为未匹配，请确认后安装。
        </div>

        <div className="import-head">
          <span className="col-check" />
          <span className="col-name">MOD 名称</span>
          <span className="col-char">安装位置</span>
        </div>

        <div className="import-list">
          {rows.map((r, i) => (
            <div className="import-row" key={`${r.path}-${i}`}>
              <input
                type="checkbox"
                className="col-check"
                checked={r.selected}
                onChange={(e) => {
                  const next = [...rows];
                  next[i] = { ...r, selected: e.target.checked };
                  setRows(next);
                }}
              />
              <div className="col-name">
                <input
                  className="import-name-input"
                  value={r.name}
                  onChange={(e) => {
                    const next = [...rows];
                    next[i] = { ...r, name: e.target.value };
                    setRows(next);
                  }}
                />
                <div className="import-file" title={r.path}>
                  {/\.(zip|rar|7z)$/i.test(r.path) ? "" : "📁 "}
                  {r.path.split(/[\\/]/).pop()}
                </div>
              </div>
              <div className="col-char">
                <CharacterPicker
                  value={r.character}
                  characters={chars}
                  onChange={(name) => pickCharacter(i, name)}
                />
              </div>
            </div>
          ))}
        </div>

        <div className="modal-foot">
          <span className="modal-stat">
            共 {rows.length} 项 · 已选 {selectedCount} 项可安装
          </span>
          <div className="modal-actions">
            <button onClick={onCancel}>取消</button>
            <button
              className="primary"
              disabled={busy || selectedCount === 0}
              onClick={() =>
                onConfirm(
                  rows
                    .filter((r) => r.selected && r.character)
                    .map((r) => ({
                      path: r.path,
                      character: r.character,
                      name: r.name,
                    }))
                )
              }
            >
              {busy ? "安装中…" : `开始安装 (${selectedCount})`}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ===== 整合包导入弹窗 =====
function ModpackDialog({
  onClose,
  onDone,
}: {
  onClose: () => void;
  onDone: (r: ModpackResult) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [picking, setPicking] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [info, setInfo] = useState<ModpackInfo | null>(null);
  const [err, setErr] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number; name: string } | null>(
    null
  );

  // 导入进度（后端逐项 emit）
  useEffect(() => {
    let alive = true;
    let un: (() => void) | undefined;
    listen<{ current: number; total: number; name: string }>("modpack-progress", (e) => {
      if (alive) setProgress(e.payload);
    }).then((u) => {
      un = u;
    });
    return () => {
      alive = false;
      un?.();
    };
  }, []);

  async function analyze(source: string) {
    setErr("");
    setBusy(true);
    setProgress(null);
    try {
      const res = await invoke<ModpackInfo>("modpack_analyze", { source });
      setInfo(res);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      setPicking(false);
    }
  }

  async function pickFolder() {
    setPicking(true);
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择整合包文件夹（含 Mods\\角色\\具体Mod）",
      });
      if (typeof picked === "string") await analyze(picked);
      else setPicking(false);
    } catch (e) {
      setErr(String(e));
      setPicking(false);
    }
  }

  async function pickArchive() {
    setPicking(true);
    try {
      const picked = await open({
        multiple: false,
        title: "选择整合包压缩包",
        filters: [{ name: "整合包", extensions: ["zip", "7z", "rar"] }],
      });
      if (typeof picked === "string") await analyze(picked);
      else setPicking(false);
    } catch (e) {
      setErr(String(e));
      setPicking(false);
    }
  }

  /** 切换识别层级 */
  async function switchLevel(path: string) {
    if (!info || path === info.mods_root) return;
    setErr("");
    setSwitching(true);
    try {
      const res = await invoke<ModpackInfo>("modpack_inspect", {
        modsRoot: path,
        tempDir: info.temp_dir,
        source: info.source,
      });
      setInfo(res);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSwitching(false);
    }
  }

  async function startImport() {
    if (!info) return;
    setErr("");
    setBusy(true);
    try {
      const r = await invoke<ModpackResult>("modpack_import", {
        modsRoot: info.mods_root,
        tempDir: info.temp_dir,
        overwrite,
      });
      onDone(r);
    } catch (e) {
      setErr(String(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  }

  function cancel() {
    if (busy) return;
    // 压缩包解压出的临时目录要清掉
    if (info?.temp_dir) {
      invoke("modpack_cleanup", { path: info.temp_dir }).catch(() => {});
    }
    onClose();
  }

  const totalItems = info ? info.mod_count + info.loose_file_count : 0;
  const pct =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.current / progress.total) * 100))
      : 0;

  return (
    <div className="modal-overlay" onClick={cancel}>
      <div className="modal pack-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>🧩 整合包导入</h3>
          <button className="modal-close" onClick={cancel} disabled={busy}>
            ✕
          </button>
        </div>

        {!info ? (
          <>
            <div className="modal-tip">
              按「<strong>Mods\角色\具体Mod</strong>」三层结构识别，整包覆盖到当前 Mods 目录。
              支持选择解压后的文件夹，也可以直接选 zip / 7z / rar（会自动解压）。
            </div>
            <div className="pack-choose">
              <button className="pack-choice" onClick={pickFolder} disabled={picking || busy}>
                <span className="pack-choice-icon">📁</span>
                <span className="pack-choice-title">选择文件夹</span>
                <span className="pack-choice-desc">含 Mods 文件夹，或直接是角色层目录</span>
              </button>
              <button className="pack-choice" onClick={pickArchive} disabled={picking || busy}>
                <span className="pack-choice-icon">🗜️</span>
                <span className="pack-choice-title">选择压缩包</span>
                <span className="pack-choice-desc">zip / 7z / rar，自动解压后识别</span>
              </button>
            </div>
            {picking && !busy && (
              <div className="pack-hint">正在等待选择文件…</div>
            )}
            {busy && <div className="pack-hint">正在识别整合包（压缩包会先解压，请稍候）…</div>}
            {err && <div className="pack-error">{err}</div>}
          </>
        ) : (
          <>
            <div className="modal-tip">
              来源：<span className="pack-path" title={info.source}>{info.source}</span>
            </div>

            <div className="pack-body">
              <div className="pack-stats">
                <div className="pack-stat">
                  <b>{info.character_count}</b>
                  <span>角色</span>
                </div>
                <div className="pack-stat">
                  <b>{info.mod_count}</b>
                  <span>Mod</span>
                </div>
                <div className={`pack-stat ${info.conflict_count ? "warn" : ""}`}>
                  <b>{info.conflict_count}</b>
                  <span>同名冲突</span>
                </div>
                <div className="pack-stat">
                  <b>{info.loose_file_count}</b>
                  <span>散文件</span>
                </div>
              </div>

              {info.candidates.length > 1 && (
                <label className="pack-level">
                  <span>识别层级</span>
                  <select
                    value={info.mods_root}
                    disabled={switching || busy}
                    onChange={(e) => switchLevel(e.target.value)}
                  >
                    {info.candidates.map((c) => (
                      <option key={c.path} value={c.path}>
                        {c.label}（{c.character_count} 角色 / {c.mod_count} Mod）
                      </option>
                    ))}
                  </select>
                </label>
              )}

              <div className="pack-chars">
                {info.characters.map((c) => (
                  <div className="pack-char" key={c.name}>
                    <div className="pack-char-head">
                      <span className="pack-char-name">{c.name}</span>
                      {c.exists && <span className="pack-badge exists">已存在</span>}
                      <span className="pack-char-count">{c.mod_count} Mod</span>
                    </div>
                    <div className="pack-entries">
                      {c.entries.map((e) => (
                        <span
                          key={`${c.name}-${e.name}`}
                          className={`pack-entry ${e.kind === "file" ? "file" : ""} ${
                            e.conflict ? "conflict" : ""
                          } ${e.disabled ? "disabled" : ""}`}
                          title={
                            e.kind === "file"
                              ? "角色目录下的文件"
                              : e.disabled
                              ? "包内为停用状态（保留 DISABLED_ 前缀）"
                              : "Mod 目录"
                          }
                        >
                          {e.name}
                          {e.conflict && <em>同名</em>}
                        </span>
                      ))}
                    </div>
                  </div>
                ))}
              </div>

              {info.conflict_count > 0 && (
                <label className="pack-overwrite">
                  <input
                    type="checkbox"
                    checked={overwrite}
                    disabled={busy}
                    onChange={(e) => setOverwrite(e.target.checked)}
                  />
                  <span>
                    覆盖同名的 <b>{info.conflict_count}</b> 项（旧目录移到回收站，可还原）；
                    不勾选则跳过这些同名 Mod
                  </span>
                </label>
              )}

              {busy && (
                <div className="pack-progress">
                  <div className="pack-progress-bar">
                    <div className="pack-progress-fill" style={{ width: `${pct}%` }} />
                  </div>
                  <div className="pack-progress-text">
                    <span>
                      {progress?.current ?? 0} / {progress?.total ?? totalItems}
                    </span>
                    <span className="pack-progress-name">{progress?.name || "准备中…"}</span>
                  </div>
                </div>
              )}

              {err && <div className="pack-error">{err}</div>}
            </div>

            <div className="modal-foot">
              <span className="modal-stat">
                将导入 {info.character_count} 个角色 · {totalItems} 项内容
              </span>
              <div className="modal-actions">
                <button onClick={cancel} disabled={busy}>
                  取消
                </button>
                <button className="primary" disabled={busy || totalItems === 0} onClick={startImport}>
                  {busy ? "导入中…" : `开始导入`}
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ===== 角色一览视图 =====
/** 文档坐标矩形（相对页面，不受容器滚动影响） */
type DocRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
};

function CharactersView({
  groups,
  gameId,
  skinsCatalog,
  modsPath,
  selectedChar,
  onSelect,
  onRename,
  onAddCharacter,
  onPickArchives,
  onOpenPack,
  onRefreshAll,
  cardSize,
  updateKeys,
  checkingUpdates,
  onCheckUpdates,
}: {
  groups: CharacterGroup[];
  gameId: string;
  skinsCatalog: Record<string, { characters: any[] }>;
  modsPath: string;
  selectedChar: string;
  onSelect: (c: string) => void;
  onRename: (oldName: string, newName: string) => void;
  onAddCharacter: (name: string) => void;
  onPickArchives: () => void;
  onOpenPack: () => void;
  onRefreshAll: () => void;
  cardSize: number;
  /** 有新版本的市场安装 Mod（rel_path 集合） */
  updateKeys: string[];
  /** 更新检测进行中 */
  checkingUpdates: boolean;
  /** 触发更新检测 */
  onCheckUpdates: (force?: boolean) => void;
}) {
  const [editing, setEditing] = useState<string>("");
  const [editValue, setEditValue] = useState("");
  const [addingChar, setAddingChar] = useState(false);
  const [newCharName, setNewCharName] = useState("");
  const [search, setSearch] = useState("");
  const [coverBust, setCoverBust] = useState<Record<string, number>>({});
  const [coverBusy, setCoverBusy] = useState<string>("");

  /** 更换角色封面：选择图片 → 复制为 _cover.png */
  async function handlePickCover(charName: string) {
    if (coverBusy) return;
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] }],
      });
      if (typeof picked !== "string") return;
      setCoverBusy(charName);
      await invoke("set_character_cover", {
        character: charName,
        imagePath: picked,
      });
      setCoverBust((prev) => ({ ...prev, [charName]: Date.now() }));
      onRefreshAll();
    } catch (e) {
      alert(`更换封面失败：${String(e)}`);
    } finally {
      setCoverBusy("");
    }
  }
  const [filter, setFilter] = useState<"all" | "hasMods" | "hasEnabled" | "hasUpdates">("all");
  const [sortBy, setSortBy] = useState<"recent" | "mods" | "name" | "custom">("recent");
  // 角色自定义顺序（拖拽排序后保存到 config.json）
  const [charOrder, setCharOrder] = useState<string[]>([]);
  // 拖拽排序进行中：源卡片名 / 插入目标（卡片名 + 方向）
  const [dragName, setDragName] = useState<string | null>(null);
  // side='left'：源插到该卡前面（该卡往右挤）；side='right'：源插到该卡后面（该卡往左挤）
  const [dragTarget, setDragTarget] = useState<{ name: string; side: "left" | "right" } | null>(null);
  // 拖拽结束后短暂屏蔽 click（避免拖拽误触发进入详情页）
  const [justDragged, setJustDragged] = useState(false);
  // 鼠标拖拽排序：mousedown 起点（超过移动阈值才视为拖拽，否则是普通点击）
  const dragStartRef = useRef<{ name: string; x: number; y: number } | null>(null);

  // 加载角色自定义顺序
  useEffect(() => {
    invoke<string[]>("get_character_order")
      .then((order) => setCharOrder(order || []))
      .catch(() => {});
  }, []);

  const query = search.trim().toLowerCase();
  let visibleGroups = groups.filter((g) => {
    if (query) {
      const hitName = g.name.toLowerCase().includes(query);
      const hitPy =
        (g.pinyin || "").includes(query) ||
        (g.pinyin_initials || "").includes(query);
      if (!hitName && !hitPy) return false;
    }
    if (filter === "hasMods") return g.total_count > 0;
    if (filter === "hasEnabled") return g.enabled_count > 0;
    if (filter === "hasUpdates") {
      // 角色（或其分区）下存在有新版本的市场安装 Mod
      return g.mods.some((m) => updateKeys.includes(m.rel_path)) ||
        g.mods.some((m) => updateKeys.includes(`${g.name}/${m.folder_name}`));
    }
    return true;
  });

  // 拖拽中的实时预览顺序：把源插入到目标位置（side 决定该卡被往左还是往右挤）
  const previewOrder = (() => {
    if (!dragName) return null;
    const base = getBaseOrder();
    const fromIdx = base.indexOf(dragName);
    if (fromIdx === -1) return null;
    const next = [...base];
    next.splice(fromIdx, 1);
    if (dragTarget) {
      const toIdx = next.indexOf(dragTarget.name);
      if (toIdx !== -1) {
        // left = 插到该卡前（该卡往右挤）；right = 插到该卡后（该卡往左挤）
        next.splice(dragTarget.side === "right" ? toIdx + 1 : toIdx, 0, dragName);
      } else {
        next.splice(fromIdx, 0, dragName);
      }
    } else {
      next.splice(fromIdx, 0, dragName); // 死区/空白：源回原位
    }
    return next;
  })();

  // 排序
  visibleGroups = [...visibleGroups].sort((a, b) => {
    if (previewOrder) {
      // 拖拽中：按预览顺序排（覆盖其它排序方式，实时让位）
      return previewOrder.indexOf(a.name) - previewOrder.indexOf(b.name);
    }
    if (sortBy === "recent")
      return (
        b.enabled_count - a.enabled_count ||
        b.total_count - a.total_count ||
        a.name.localeCompare(b.name, "zh")
      );
    if (sortBy === "mods") return b.total_count - a.total_count || a.name.localeCompare(b.name, "zh");
    if (sortBy === "custom") {
      // 自定义顺序：按 charOrder 里的位置排，未列出的角色按名称排后面
      const idx = (n: string) => {
        const i = charOrder.indexOf(n);
        return i === -1 ? charOrder.length : i;
      };
      return (
        idx(a.name) - idx(b.name) ||
        a.name.localeCompare(b.name, "zh")
      );
    }
    return a.name.localeCompare(b.name, "zh");
  });

  const totalMods = groups.reduce((s, g) => s + g.total_count, 0);
  const totalSkins = groups.reduce(
    (s, g) => s + findCharacterSkins(skinsCatalog, gameId, g.name).length,
    0
  );

  function startEdit(name: string) {
    setEditing(name);
    setEditValue(name);
  }

  function commitEdit() {
    const target = editing;
    const next = editValue.trim();
    setEditing("");
    if (target && next && next !== target) {
      onRename(target, next);
    }
  }

  function commitAdd() {
    const name = newCharName.trim();
    setAddingChar(false);
    setNewCharName("");
    if (name) onAddCharacter(name);
  }

  // ===== 角色卡片拖拽排序（鼠标事件自绘，不用 HTML5 DnD） =====
  // 背景：Tauri 的 dragDropEnabled(默认 true) 会拦截 WebView2 原生拖放路径，
  // 导致 HTML5 dragstart/dragover/drop 在真实鼠标操作下不触发（显示禁止光标）。
  // 拖入 zip 导入又依赖 Tauri 原生拖放，因此卡片排序改用 mousedown/mousemove/mouseup 实现。
  // 动效：幽灵卡片跟手（DOM 直操） + 实时预览顺序 + FLIP 让位动画。

  // 幽灵卡片（克隆源卡片，fixed 定位跟手；不进 React 状态保证流畅）
  const ghostRef = useRef<{
    el: HTMLElement;
    baseX: number;
    baseY: number;
    offX: number;
    offY: number;
    /** 幽灵卡片尺寸（= 源卡片大小，让位判定范围） */
    w: number;
    h: number;
    /** 当前光标位置（推算幽灵矩形用） */
    curX: number;
    curY: number;
  } | null>(null);
  // 每张卡片的 DOM 引用（FLIP 动画测位置用）
  const cardRefs = useRef(new Map<string, HTMLElement>());
  // 滚动容器（.view）引用：槽位用文档坐标，滚动偏移从这里取
  const viewRef = useRef<HTMLDivElement | null>(null);
  // 上一次渲染的卡片位置快照（FLIP 旧位置，文档坐标）
  const prevRects = useRef(new Map<string, DocRect>());
  // 本轮槽位矩形快照（纯布局位置的文档坐标，让位判定用——与动画解耦防震荡、滚动不影响）
  const slotRectsRef = useRef(new Map<string, DocRect>());
  // FLIP 统一清理 timer（新一轮作废旧轮，防竞态残留）
  const flipTimerRef = useRef<number | undefined>(undefined);
  // dragTarget 的 ref 镜像（onUp 读取用：effect 依赖不含 dragTarget，
  // 闭包里的 state 是拖拽开始时的旧值，必须走 ref 才能拿到最新判定）
  const dragTargetRef = useRef<{ name: string; side: "left" | "right" } | null>(null);

  /** 按下卡片：记录起点（左键、且不在按钮/输入框上）。
   *  拖拽排序仅在「自定义」排序下可用，其它排序按下视为普通点击。 */
  function handleCardMouseDown(e: React.MouseEvent, name: string) {
    if (e.button !== 0) return;
    if (sortBy !== "custom") return; // 仅自定义排序可拖拽
    const t = e.target as HTMLElement;
    if (t.closest("button, input, textarea, .cover-edit-btn")) return;
    if (editing === name) return;
    dragStartRef.current = { name, x: e.clientX, y: e.clientY };
  }

  /** 基础顺序：优先用已有 charOrder，未列出的补后面（按名称），剔除失效名 */
  function getBaseOrder(): string[] {
    const allNames = groups.map((g) => g.name);
    const base =
      charOrder.length > 0 ? [...charOrder] : [...allNames].sort((a, b) => a.localeCompare(b, "zh"));
    for (const n of allNames) {
      if (!base.includes(n)) base.push(n);
    }
    return base.filter((n) => allNames.includes(n));
  }

  /** 拖拽重排核心：把 source 插到 target 的 side 侧并持久化。
   *  left = 该卡往右挤（源在其前）；right = 该卡往左挤（源在其后）。
   *  注意：不自动切换排序方式——「自定义」必须手动选择（仅自定义下可拖拽）。 */
  function commitDragOrder(
    source: string,
    target: { name: string; side: "left" | "right" }
  ) {
    const validBase = getBaseOrder();
    const fromIdx = validBase.indexOf(source);
    if (fromIdx === -1 || validBase.indexOf(target.name) === -1) return;

    const next = [...validBase];
    next.splice(fromIdx, 1);
    // 移除源后重新定位目标卡，按 side 决定插到其前/后
    const reIdx = next.indexOf(target.name);
    const toIdx = reIdx === -1 ? next.length : reIdx + (target.side === "right" ? 1 : 0);
    next.splice(toIdx, 0, source);

    setCharOrder(next);
    invoke("set_character_order", { order: next }).catch(() => {});
  }

  /** 创建幽灵卡片（克隆源卡片 DOM，fixed 定位跟手）。兜底：先清掉任何残留 ghost，保证单例 */
  function createGhost(source: string, x: number, y: number) {
    document.querySelectorAll(".char-ghost").forEach((el) => el.remove());
    const srcEl = cardRefs.current.get(source);
    if (!srcEl) return;
    const rect = srcEl.getBoundingClientRect();
    const ghost = srcEl.cloneNode(true) as HTMLElement;
    ghost.className = "char-card char-ghost";
    ghost.style.width = `${rect.width}px`;
    ghost.style.height = `${rect.height}px`;
    ghost.style.left = `${rect.left}px`;
    ghost.style.top = `${rect.top}px`;
    document.body.appendChild(ghost);
    // 下一帧加浮起状态（scale + 阴影，走 CSS 过渡）
    requestAnimationFrame(() => ghost.classList.add("char-ghost-active"));
    ghostRef.current = {
      el: ghost,
      baseX: rect.left,
      baseY: rect.top,
      // offX/offY 基于按下点（st.x/st.y），不是触发拖拽的 mousemove 位置——
      // 否则抓取点会漂移（阈值 6px 的两倍），ghost 视觉中心与判定中心都对不准
      offX: x - rect.left,
      offY: y - rect.top,
      w: rect.width,
      h: rect.height,
      curX: x,
      curY: y,
    };
  }

  /** 移动幽灵卡片到光标处（保持抓取点偏移） */
  function moveGhost(x: number, y: number) {
    const g = ghostRef.current;
    if (!g) return;
    g.curX = x;
    g.curY = y;
    const dx = x - g.offX - g.baseX;
    const dy = y - g.offY - g.baseY;
    g.el.style.transform = `translate(${dx}px, ${dy}px)`;
  }

  /** 卡片中央死区宽度（竖条，约一个汉字）：ghost 中心线压入时不改变让位状态 */
  const CARD_DEAD_ZONE_W = 22;

  /** 让位判定（与动画解耦，用槽位矩形的文档坐标）：按 ghost 中心点定位所在槽位。
   *  返回 "keep"（保持当前让位状态，不回位）或有效目标 { name, side }：
   *  - 中心点未命中任何槽位（空白/gap）→ "keep"
   *  - 命中源自己的占位槽位 → "keep"
   *  - 命中槽位中央死区 → "keep"
   *  - 命中槽位死区左侧（卡片左半）→ { name, side:'left' }（该卡往右挤）
   *  - 命中槽位死区右侧（卡片右半）→ { name, side:'right' }（该卡往左挤）
   *  拖拽全程让位状态锁定：只有进入有效区才更新目标，防「经过死区归位再挤开」的闪烁。 */
  function ghostTarget(): { name: string; side: "left" | "right" } | "keep" {
    const g = ghostRef.current;
    if (!g) return "keep";
    const viewEl = viewRef.current;
    const sl = viewEl?.scrollLeft ?? 0;
    const st = viewEl?.scrollTop ?? 0;
    // ghost 中心点（文档坐标）
    const centerX = g.curX + sl - g.offX + g.w / 2;
    const centerY = g.curY + st - g.offY + g.h / 2;
    let hit: { name: string; rect: DocRect } | null = null;
    for (const [name, r] of slotRectsRef.current) {
      if (centerX >= r.left && centerX < r.right && centerY >= r.top && centerY < r.bottom) {
        hit = { name, rect: r };
        break;
      }
    }
    if (!hit) return "keep"; // 空白/gap：保持现状
    const { name, rect } = hit;
    if (name === dragName) return "keep"; // 自己占位：保持现状
    const cardCx = (rect.left + rect.right) / 2;
    if (Math.abs(centerX - cardCx) <= CARD_DEAD_ZONE_W / 2) return "keep"; // 死区：保持现状
    return { name, side: centerX < cardCx ? "left" : "right" };
  }

  /** 销毁幽灵卡片（全量清除，防残留悬空） */
  function destroyGhost() {
    document.querySelectorAll(".char-ghost").forEach((el) => el.remove());
    ghostRef.current = null;
  }

  // 全局 mousemove / mouseup：拖拽判定 + 悬停目标识别 + 放下提交
  useEffect(() => {
    function cleanup() {
      dragStartRef.current = null;
      document.body.classList.remove("char-dragging");
      destroyGhost();
    }

    function onMove(e: MouseEvent) {
      const st = dragStartRef.current;
      if (!st) return;
      if (!dragName) {
        // 未进入拖拽模式：移动超过阈值才判定为拖拽（否则视为点击）
        const dist = Math.hypot(e.clientX - st.x, e.clientY - st.y);
        if (dist > 6) {
          setDragName(st.name);
          document.body.classList.add("char-dragging");
          // 用按下位置创建幽灵卡片（offX/offY 按真实抓取点计算，防 15px 漂移）
          createGhost(st.name, st.x, st.y);
        }
        return;
      }
      // 拖拽中：幽灵跟手 + 槽位/死区/方向判定
      moveGhost(e.clientX, e.clientY);
      const t = ghostTarget();
      // "keep"（死区/自己占位/空白）：保持当前让位状态不变——拖拽全程锁定，
      // 被挤开的卡片不回位，只有进入新的有效区才更新目标（防高速移动闪烁）
      if (t === "keep") return;
      const prev = dragTargetRef.current;
      // 判定结果与当前一致则不更新（减少无谓渲染）
      if (prev && prev.name === t.name && prev.side === t.side) return;
      dragTargetRef.current = t; // ref 同步更新（onUp 依赖它）
      setDragTarget(t);
    }

    function onUp() {
      const wasDragging = !!dragName;
      const source = dragName;
      // 松手直接采用最后一次判定（ref 镜像）——所见即所得
      const target = dragTargetRef.current;
      cleanup();
      dragTargetRef.current = null;
      setDragName(null);
      setDragTarget(null);
      if (!wasDragging) return; // 没有进入拖拽模式（普通点击），交给 click 处理
      // 拖拽结束后短暂屏蔽 click，避免误触发进入详情页
      setJustDragged(true);
      window.setTimeout(() => setJustDragged(false), 300);
      if (source && target) {
        commitDragOrder(source, target);
      }
    }

    // 注：不监听 document mouseleave——WebView2/CDP 环境下它会误触发并中断拖拽
    // （cleanup 销毁幽灵导致拖拽失效）。移出窗口后回到窗口时 mousemove 会继续判定，
    // mouseup 挂在 window 上在窗口内松手也能收到，无需此保险。

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      // 注意：这里不销毁 ghost —— dragName 变化触发 effect 重绑时，
      // 刚创建的幽灵卡片会被误删（ghost 生命周期由 onUp/组件卸载管理）
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragName, charOrder, groups]);

  // 组件卸载时销毁幽灵卡片
  useEffect(() => () => destroyGhost(), []);

  // FLIP 让位动画：卡片因预览顺序变化而重排时，从旧位置平滑滑到新位置
  // 关键：判定用的「槽位矩形」在这里测量（纯布局位置，先清掉动画残留的 transform），
  // 与 FLIP 动画完全解耦——否则动画中的位置会反馈进判定，造成来回震荡（乱闪）。
  useLayoutEffect(() => {
    const animating = !!dragName || justDragged;
    // 作废上一轮的统一清理 timer，并清掉所有残留内联样式
    // （同步序列：清→测→写反向，都在浏览器渲染前完成，无视觉跳变）
    if (flipTimerRef.current) window.clearTimeout(flipTimerRef.current);
    cardRefs.current.forEach((el) => {
      el.style.transition = "none";
      el.style.transform = "";
    });
    // 测量本轮槽位（文档坐标 = 视口坐标 + .view 滚动偏移；滚动不影响文档坐标，
    // 只有布局变化才需要重测，解决滚动后判定错位）
    const viewEl = viewRef.current;
    const sl = viewEl?.scrollLeft ?? 0;
    const st = viewEl?.scrollTop ?? 0;
    const slotRects = new Map<string, DocRect>();
    cardRefs.current.forEach((el, name) => {
      const r = el.getBoundingClientRect();
      slotRects.set(name, {
        left: r.left + sl,
        top: r.top + st,
        right: r.right + sl,
        bottom: r.bottom + st,
        width: r.width,
        height: r.height,
      });
    });
    slotRectsRef.current = slotRects;

    if (!animating) {
      // 非拖拽渲染：清样式恢复常态，只更新快照
      cardRefs.current.forEach((el) => {
        el.style.transition = "";
      });
      prevRects.current = slotRects;
      return;
    }

    // FLIP：位置变化（相对上一轮槽位）的卡片滑过去（源卡片隐身，不动画）
    cardRefs.current.forEach((el, name) => {
      if (name === dragName) return;
      const next = slotRects.get(name)!;
      const prev = prevRects.current.get(name);
      if (!prev) return;
      const dx = prev.left - next.left;
      const dy = prev.top - next.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
      el.style.transform = `translate(${dx}px, ${dy}px)`;
      requestAnimationFrame(() => {
        el.style.transition = "transform 0.3s cubic-bezier(0.25, 0.8, 0.35, 1)";
        el.style.transform = "";
      });
    });
    // 统一清理（新一轮 FLIP 会作废此 timer，避免竞态残留）
    flipTimerRef.current = window.setTimeout(() => {
      cardRefs.current.forEach((el) => {
        el.style.transition = "";
        el.style.transform = "";
      });
    }, 380);
    prevRects.current = slotRects;
  });

  return (
    <div className="view" ref={viewRef}>
      <div className="char-header-panel">
        <div className="char-header-top">
          <div>
            <h2>角色列表</h2>
            <div className="subtitle">
              当前显示 {visibleGroups.length} / {groups.length} 个角色，合计 {totalMods} 个
              Mod · {totalSkins} 款皮肤
            </div>
          </div>
          <div className="char-header-actions">
            <button className="panel-btn" onClick={onPickArchives}>
              📦 批量导入
            </button>
            <button
              className="panel-btn"
              onClick={onOpenPack}
              title="导入带 Mods/角色/具体Mod 结构的整合包"
            >
              🧩 整合包导入
            </button>
            <button className="panel-btn repair" disabled title="开发中，敬请期待">
              🛠 修复 Mod
            </button>
            <button className="panel-btn" onClick={onRefreshAll}>
              刷新 / 整理
            </button>
          </div>
        </div>

        <div className="char-toolbar">
          <div className="search-box char-search">
            <span className="search-icon">🔍</span>
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="搜索角色名 / 英文 / 拼音首字母..."
            />
          {search && (
            <button className="search-clear" onClick={() => setSearch("")}>
              ✕
            </button>
          )}
        </div>

        <div className="filter-group">
          <span className="group-label">显示</span>
          {(
            [
              ["hasMods", "有内容"],
              ["all", "全部角色"],
              ["hasEnabled", "启用中"],
              ["hasUpdates", checkingUpdates ? "检测中…" : "有更新"],
            ] as const
          ).map(([k, label]) => (
            <button
              key={k}
              className={`sort-chip ${filter === k ? "active" : ""}`}
              onClick={() => {
                setFilter(k);
                if (k === "hasUpdates") onCheckUpdates();
              }}
              title={k === "hasUpdates" ? "显示检测到有新版本的市场安装 Mod" : undefined}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="filter-group">
          <span className="group-label">排序</span>
          {(
            [
              ["recent", "常用"],
              ["mods", "Mod 数量"],
              ["name", "名称"],
              ["custom", "自定义"],
            ] as const
          ).map(([k, label]) => (
            <button
              key={k}
              className={`sort-chip ${sortBy === k ? "active" : ""}`}
              onClick={() => setSortBy(k)}
              title={k === "custom" ? "按手动拖拽的顺序排列（此排序下可拖拽卡片调整顺序）" : undefined}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      </div>

      <div
        className={`char-grid ${sortBy === "custom" ? "char-custom" : ""}`}
        style={{ "--card-w": `${cardSize}px` } as React.CSSProperties}
      >
        {/* 新增角色卡片（置于最前） */}
        {addingChar ? (
          <div className="char-card char-card-add editing">
            <input
              autoFocus
              className="add-char-input"
              value={newCharName}
              onChange={(e) => setNewCharName(e.target.value)}
              placeholder="角色名称"
              maxLength={40}
              onBlur={() => {
                setAddingChar(false);
                setNewCharName("");
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitAdd();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setAddingChar(false);
                  setNewCharName("");
                }
              }}
            />
            <span className="add-char-hint">回车创建 · Esc 取消</span>
          </div>
        ) : (
          <div
            className="char-card char-card-add"
            onClick={() => {
              setAddingChar(true);
              setNewCharName("");
            }}
          >
            <span className="add-char-plus">＋</span>
            <span className="add-char-label">新增角色</span>
          </div>
        )}

        {visibleGroups.map((g) => {
          const skins = findCharacterSkins(skinsCatalog, gameId, g.name);
          return (
            <div
              key={g.name}
              ref={(el) => {
                if (el) cardRefs.current.set(g.name, el);
                else cardRefs.current.delete(g.name);
              }}
              className={`char-card ${g.name === selectedChar ? "active" : ""} ${
                dragName === g.name ? "dragging" : ""
              }`}
              onMouseDown={(e) => handleCardMouseDown(e, g.name)}
              onClick={() => {
                if (justDragged) return;
                if (editing !== g.name) onSelect(g.name);
              }}
            >
              <div className="char-cover">
                {(() => {
                  const updCount = g.mods.filter((m) => updateKeys.includes(m.rel_path)).length;
                  return updCount > 0 ? (
                    <span className="char-update-badge" title={`${updCount} 个 Mod 有新版本`}>
                      ⬆ {updCount}
                    </span>
                  ) : null;
                })()}
                <Cover
                  basePath={modsPath}
                  character={g.name}
                  cover={g.cover}
                  bust={coverBust[g.name]}
                  gameId={gameId}
                />

                {/* 更换封面按钮（悬浮显示） */}
                <button
                  className="cover-edit-btn"
                  title="更换角色封面（自动保存为 _cover.png）"
                  onClick={(e) => {
                    e.stopPropagation();
                    handlePickCover(g.name);
                  }}
                >
                  {coverBusy === g.name ? "…" : "📷"}
                </button>

                {/* 左下角：皮肤数量 */}
                <span className="cover-badge skin" title="官方皮肤数量">
                  <span className="badge-icon">👕</span>
                  {skins.length}
                </span>
                {/* 右下角：Mod 数量 */}
                <span className="cover-badge mod" title={`${g.enabled_count} 启用 / ${g.total_count} 总数`}>
                  <span className="badge-icon">📦</span>
                  {g.enabled_count}/{g.total_count}
                </span>
              </div>

              {editing === g.name ? (
                <input
                  className="char-name-input"
                  autoFocus
                  value={editValue}
                  onChange={(e) => setEditValue(e.target.value)}
                  onClick={(e) => e.stopPropagation()}
                  onBlur={() => setEditing("")}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitEdit();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      setEditing("");
                    }
                  }}
                  maxLength={40}
                />
              ) : (
                <div className="char-card-name">
                  <span className="name-text">{g.name}</span>
                  <button
                    className="name-edit-btn"
                    title="重命名角色"
                    onClick={(e) => {
                      e.stopPropagation();
                      startEdit(g.name);
                    }}
                  >
                    ✏️
                  </button>
                </div>
              )}
            </div>
          );
        })}

        {visibleGroups.length === 0 && query && (
          <div className="empty-hint char-search-empty">
            没有匹配「{search}」的角色
          </div>
        )}
      </div>
    </div>
  );
}

// ===== 角色详情视图（仿 QAQM：中栏 Mod 列表 + 右栏详情） =====
function CharacterDetailView({
  group,
  allCharacterNames,
  gameId,
  skinsCatalog,
  modsPath,
  onToggle,
  onImport,
  onBack,
  busy,
  onRefresh,
  galleryCardSize,
  setMsg,
  setError,
}: {
  group: CharacterGroup;
  /** 全部角色名（Mod 移动的目标选择列表） */
  allCharacterNames: string[];
  gameId: string;
  skinsCatalog: Record<string, { characters: any[] }>;
  modsPath: string;
  onToggle: (m: ModEntry) => Promise<void> | void;
  onImport: () => void;
  onBack: () => void;
  busy: boolean;
  onRefresh: () => void;
  galleryCardSize: number;
  setMsg: (s: string) => void;
  setError: (s: string) => void;
}) {
  const [selectedName, setSelectedName] = useState<string>("");
  const [dragOver, setDragOver] = useState(false);
  const [coverBust, setCoverBust] = useState(Date.now());
  const [saveMsg, setSaveMsg] = useState("");
  const [saving, setSaving] = useState(false);
  const [search, setSearch] = useState("");
  const [sortBy, setSortBy] = useState<"name" | "status" | "time" | "custom">("name");
  // Mod 移动（装错角色的纠正）：来源 mod + 目标角色
  const [moving, setMoving] = useState<ModEntry | null>(null);
  const [moveTarget, setMoveTarget] = useState("");
  const [moveBusy, setMoveBusy] = useState(false);

  async function handleMoveMod() {
    if (!moving || !moveTarget) return;
    setMoveBusy(true);
    try {
      const newRel = await invoke<string>("move_mod", {
        fromCharacter: moving.character,
        folderName: moving.folder_name,
        toCharacter: moveTarget,
      });
      setMsg(`已移动到 ${moveTarget}（${newRel}）`);
      setMoving(null);
      setMoveTarget("");
      onRefresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setMoveBusy(false);
    }
  }
  const [singleMode, setSingleMode] = useState(
    () => localStorage.getItem("qaqm.singleMode") === "1"
  );

  // 单选模式持久化到 config.json（localStorage 仅作首帧兜底，EBWebView 清缓存也不丢）
  useEffect(() => {
    invoke<boolean>("get_single_mode")
      .then((v) => {
        setSingleMode(v);
        localStorage.setItem("qaqm.singleMode", v ? "1" : "0");
      })
      .catch(() => {});
  }, []);

  function handleSingleModeChange(next: boolean) {
    setSingleMode(next);
    localStorage.setItem("qaqm.singleMode", next ? "1" : "0");
    invoke("set_single_mode", { enabled: next }).catch(() => {});
  }


  // 备注 / 置顶 / 快捷键
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [pinned, setPinned] = useState<string[]>([]);
  const [hotkeys, setHotkeys] = useState<ModHotkey[] | null>(null);
  const [editingNote, setEditingNote] = useState(false);
  const [noteDraft, setNoteDraft] = useState("");
  const [renamingMod, setRenamingMod] = useState(false);
  const [renameModVal, setRenameModVal] = useState("");
  // 快捷键编辑：section + 已捕获的新按键
  const [hkEditing, setHkEditing] = useState<{ section: string; desc: string } | null>(null);
  const [hkCaptured, setHkCaptured] = useState("");

  const selRel = selectedName ? `${group.name}/${selectedName}` : "";
  const selNote = notes[selRel] || "";
  const selPinned = pinned.includes(selRel);

  // 快捷键随选中 mod 加载
  useEffect(() => {
    if (!selectedName) {
      setHotkeys(null);
      return;
    }
    let alive = true;
    invoke<ModHotkey[]>("get_mod_hotkeys", {
      character: group.name,
      modName: selectedName,
    })
      .then((list) => alive && setHotkeys(list))
      .catch(() => alive && setHotkeys([]));
    return () => {
      alive = false;
    };
  }, [group.name, selectedName, coverBust]);

  // 快捷键捕获：编辑模式下拦截键盘输入（QAQM 同款映射）
  useEffect(() => {
    if (!hkEditing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setHkEditing(null);
        setHkCaptured("");
        return;
      }
      if (e.key === "Enter" && hkCaptured) {
        saveHotkey();
        return;
      }
      const parts: string[] = [];
      if (e.ctrlKey) parts.push("ctrl");
      if (e.altKey) parts.push("alt");
      if (e.shiftKey) parts.push("shift");
      const key = e.key.toUpperCase();
      if (key !== "CONTROL" && key !== "ALT" && key !== "SHIFT") {
        const keyMap: Record<string, string> = {
          F1: "F1", F2: "F2", F3: "F3", F4: "F4", F5: "F5", F6: "F6",
          F7: "F7", F8: "F8", F9: "F9", F10: "F10", F11: "F11", F12: "F12",
          ESCAPE: "Esc", ENTER: "Enter", TAB: "Tab", BACKSPACE: "Backspace",
          DELETE: "Delete", ARROWUP: "Up", ARROWDOWN: "Down",
          ARROWLEFT: "Left", ARROWRIGHT: "Right",
          HOME: "Home", END: "End", PAGEUP: "PageUp", PAGEDOWN: "PageDown",
          INSERT: "Insert", SPACE: "Space", " ": "Space",
        };
        parts.push(key.length === 1 ? key : keyMap[key] || key);
      }
      if (parts.length > 0) setHkCaptured(parts.join(" + "));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hkEditing, hkCaptured, selectedName]);

  async function saveHotkey() {
    if (!hkEditing || !hkCaptured || !selectedName) return;
    const formatted = hkCaptured.replace(/\s*\+\s*/g, " ");
    try {
      await invoke("set_mod_hotkey", {
        character: group.name,
        modName: selectedName,
        section: hkEditing.section,
        newKey: formatted,
      });
      setSaveMsg(`快捷键已保存：${hkEditing.desc} → ${hkCaptured}`);
      setHkEditing(null);
      setHkCaptured("");
      setCoverBust(Date.now()); // 触发快捷键重载
    } catch (e) {
      setSaveMsg(`快捷键保存失败：${String(e)}`);
    }
  }

  async function handleTogglePin() {
    if (!selectedName) return;
    try {
      const now = await invoke<boolean>("toggle_mod_pin", {
        character: group.name,
        modName: selectedName,
      });
      setPinned((prev) => {
        const next = prev.filter((p) => p !== selRel);
        if (now) next.push(selRel);
        return next;
      });
    } catch (e) {
      setSaveMsg(`置顶失败：${String(e)}`);
    }
  }

  async function handleRenameModCommit() {
    const newName = renameModVal.trim();
    setRenamingMod(false);
    if (!newName || newName === selectedName) return;
    try {
      await invoke<string>("rename_mod", {
        character: group.name,
        oldName: selectedName,
        newName,
      });
      setSelectedName(newName);
      setSaveMsg(`已重命名为「${newName}」`);
      onRefresh();
    } catch (e) {
      setSaveMsg(`重命名失败：${String(e)}`);
    }
  }

  function saveNote() {
    setEditingNote(false);
    invoke("set_mod_note", {
      character: group.name,
      modName: selectedName,
      note: noteDraft,
    })
      .then(() => {
        setNotes((prev) => {
          const next = { ...prev };
          if (noteDraft.trim()) next[selRel] = noteDraft;
          else delete next[selRel];
          return next;
        });
      })
      .catch((e) => setSaveMsg(`备注保存失败：${String(e)}`));
  }
  const [deleting, setDeleting] = useState<ModEntry | null>(null);
  // 视图模式：列表 / 画廊（QAQM 式，详情页内切换，记忆到 localStorage）
  const [viewMode, setViewMode] = useState<"list" | "gallery">(() =>
    localStorage.getItem("qaqm.viewMode") === "gallery" ? "gallery" : "list"
  );

  // 外观分区
  const [sections, setSections] = useState<CharacterSection[]>([]);
  const [activeSection, setActiveSection] = useState<string>("base");
  const [addingSection, setAddingSection] = useState(false);
  const [newSectionName, setNewSectionName] = useState("");
  const [modSections, setModSections] = useState<Record<string, string>>({});
  const officialSkins = findCharacterSkins(skinsCatalog, gameId, group.name);

  useEffect(() => {
    (async () => {
      try {
        const list = await invoke<CharacterSection[]>("list_sections", {
          character: group.name,
        });
        setSections(list);
        const map = await invoke<Record<string, string>>("get_mod_sections");
        setModSections(map);
        const [noteMap, pinnedList] = await Promise.all([
          invoke<Record<string, string>>("get_mod_notes"),
          invoke<string[]>("get_pinned_mods"),
        ]);
        setNotes(noteMap);
        setPinned(pinnedList);
      } catch {
        setSections([]);
      }
    })();
    setActiveSection("base");
  }, [group.name]);

  // 把某个 Mod 移动到分区
  async function handleSetSection(m: ModEntry, sectionId: string) {
    try {
      await invoke("set_mod_section", {
        character: group.name,
        modName: m.name,
        sectionId,
      });
      setModSections((prev) => {
        const next = { ...prev };
        if (sectionId === "base" || !sectionId) delete next[m.rel_path];
        else next[m.rel_path] = sectionId;
        return next;
      });
    } catch (e) {
      setSaveMsg(`设置分区失败：${String(e)}`);
    }
  }


  // 当前分区下的 Mod
  const sectionMods = group.mods.filter((m) => {
    const sec = modSections[m.rel_path];
    if (activeSection === "base") return !sec;
    return sec === activeSection;
  });

  // 「原角色」分区的统计（未归属任何自定义分区的 Mod）
  const baseMods = group.mods.filter((m) => !modSections[m.rel_path]);
  const baseEnabled = baseMods.filter((m) => m.enabled).length;

  async function handleAddSection() {
    const name = newSectionName.trim();
    if (!name) {
      setAddingSection(false);
      setNewSectionName("");
      return;
    }
    try {
      const created = await invoke<CharacterSection>("add_section", {
        character: group.name,
        name,
      });
      setSections((prev) => [...prev, created]);
      setActiveSection(created.id);
      setAddingSection(false);
      setNewSectionName("");
    } catch (e) {
      setSaveMsg(`新增分类失败：${String(e)}`);
    }
  }

  async function handleDeleteSection(id: string) {
    try {
      await invoke("delete_section", { id });
      setSections((prev) => prev.filter((s) => s.id !== id));
      if (activeSection === id) setActiveSection("base");
    } catch (e) {
      setSaveMsg(`删除分类失败：${String(e)}`);
    }
  }

  // 默认选中第一个启用中的 mod，否则第一个
  useEffect(() => {
    if (group.mods.length === 0) {
      setSelectedName("");
      return;
    }
    if (!group.mods.some((m) => m.name === selectedName)) {
      const firstEnabled = group.mods.find((m) => m.enabled) || group.mods[0];
      setSelectedName(firstEnabled.name);
    }
  }, [group]);

  const selected = group.mods.find((m) => m.name === selectedName);

  // 过滤 + 排序
  const visibleMods = (() => {
    let list = sectionMods;
    const q = search.trim().toLowerCase();
    if (q) {
      list = list.filter(
        (m) =>
          m.name.toLowerCase().includes(q) ||
          m.folder_name.toLowerCase().includes(q)
      );
    }
    const sorted = [...list];
    const pinRank = (m: ModEntry) => (pinned.includes(`${group.name}/${m.name}`) ? 0 : 1);
    if (sortBy === "name") {
      sorted.sort((a, b) => pinRank(a) - pinRank(b) || a.name.localeCompare(b.name, "zh"));
    } else if (sortBy === "status") {
      sorted.sort(
        (a, b) =>
          pinRank(a) - pinRank(b) ||
          Number(b.enabled) - Number(a.enabled) ||
          a.name.localeCompare(b.name)
      );
    } else if (sortBy === "time") {
      sorted.sort((a, b) => pinRank(a) - pinRank(b) || b.modified - a.modified);
    }
    // custom 模式保持后端返回顺序，仅置顶提前
    if (sortBy === "custom") {
      sorted.sort((a, b) => pinRank(a) - pinRank(b));
    }
    return sorted;
  })();

  // 切换启用状态（单选模式下自动互斥）
  async function handleRowToggle(m: ModEntry) {
    if (singleMode && !m.enabled) {
      // 单选模式：后端一次性完成「禁用其他 + 启用目标」
      try {
        await invoke("enable_mod_exclusive", {
          character: group.name,
          modName: m.name,
        });
        onRefresh();
      } catch (e) {
        setSaveMsg(`切换失败：${String(e)}`);
      }
      return;
    }
    await onToggle(m);
  }

  async function handleOpenFolder(m: ModEntry) {
    try {
      await invoke("open_mod_folder", { character: group.name, folderName: m.folder_name });
    } catch (e) {
      setSaveMsg(`打开失败：${String(e)}`);
    }
  }

  async function handleConfirmDelete() {
    if (!deleting) return;
    const target = deleting;
    setDeleting(null);
    try {
      await invoke("delete_mod", {
        character: group.name,
        folderName: target.folder_name,
      });
      setSaveMsg(`「${target.name}」已移到回收站`);
      if (selectedName === target.name) setSelectedName("");
      onRefresh();
    } catch (e) {
      setSaveMsg(`删除失败：${String(e)}`);
    }
  }

  // 把图片写入 mod 目录作为 _cover.png
  async function applyImage(blob: Blob) {
    if (!selected || saving) return;
    setSaving(true);
    setSaveMsg("");
    try {
      const buf = await blob.arrayBuffer();
      const b64 = arrayBufferToBase64(buf);
      await invoke<string>("save_mod_cover", {
        character: group.name,
        folderName: selected.folder_name,
        base64Data: b64,
      });
      setCoverBust(Date.now());
      setSaveMsg("预览图已更新");
      onRefresh();
    } catch (e) {
      setSaveMsg(`保存失败：${String(e)}`);
    } finally {
      setSaving(false);
    }
  }

  // 全局粘贴监听（Ctrl+V 图片）
  useEffect(() => {
    function onPaste(e: ClipboardEvent) {
      if (!selected) return;
      const items = e.clipboardData?.items;
      if (!items) return;
      for (let i = 0; i < items.length; i++) {
        const item = items[i];
        if (item.type.startsWith("image/")) {
          const blob = item.getAsFile();
          if (blob) {
            e.preventDefault();
            applyImage(blob);
          }
          break;
        }
      }
    }
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [selected, saving]);

  return (
    <div className="detail-layout">
      {/* 中栏：Mod 列表 */}
      <div className="detail-main">
        <div className="detail-header">
          <button className="back-btn" onClick={onBack} title="返回角色一览">
            ←
          </button>
          <h2>{group.name}</h2>
          <span className="header-tag">原角色</span>
          <div className="detail-header-right">
            <label className="single-mode-toggle" title="开启后，同一角色下只能启用一个 Mod，启用另一个会自动禁用其他的">
              <span>单选模式</span>
              <label className="switch" onClick={(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={singleMode}
                  onChange={(e) => handleSingleModeChange(e.target.checked)}
                />
                <span className="slider" />
              </label>
            </label>
            <button
              className="gallery-btn"
              onClick={() => {
                const next = viewMode === "list" ? "gallery" : "list";
                setViewMode(next);
                localStorage.setItem("qaqm.viewMode", next);
              }}
              title={viewMode === "list" ? "切换到画廊视图（以大图卡片展示 Mod）" : "切回列表视图"}
            >
              {viewMode === "list" ? "🖼 进入画廊" : "☰ 进入列表"}
            </button>
            <button className="primary" onClick={onImport} disabled={busy}>
              {busy ? "导入中…" : "＋ 导入 Mod"}
            </button>
          </div>
        </div>

        {/* 外观分区 */}
        <div className="sections-bar">
          <div className="sections-head">
            <span className="sections-title">外观分区</span>
            <span className="sections-meta">
              {officialSkins.length} 款官方皮肤 · {1 + officialSkins.length + sections.length} 个分区
            </span>
            <span className="sections-hint">切换仅筛选显示，不会自动启停</span>
          </div>
          <div className="sections-list">
            {/* 默认分区：原角色 */}
            <div
              className={`section-card ${activeSection === "base" ? "active" : ""}`}
              onClick={() => setActiveSection("base")}
            >
              <div className="section-cover">
                {group.cover ? (
                  <Cover basePath={modsPath} character={group.name} cover={group.cover} gameId={gameId} />
                ) : (
                  <span className="section-cover-placeholder">原</span>
                )}
              </div>
              <div className="section-info">
                <div className="section-name">原角色</div>
                <div className="section-en">Original</div>
                <div className="section-stat">
                  {baseEnabled} 启用 · {baseMods.length} Mod
                </div>
              </div>
            </div>

            {/* 官方皮肤分区（QAQM 内置数据） */}
            {officialSkins.map((skin) => {
              const secMods = group.mods.filter(
                (m) => modSections[m.rel_path] === skin.sectionId
              );
              const secEnabled = secMods.filter((m) => m.enabled).length;
              return (
                <div
                  key={skin.sectionId}
                  className={`section-card ${activeSection === skin.sectionId ? "active" : ""}`}
                  onClick={() => setActiveSection(skin.sectionId)}
                  title={skin.nameEn || skin.nameZh}
                >
                  <div className="section-cover">
                    {skin.image ? (
                      <img src={skin.image} alt={skin.nameZh} className="cover-img" />
                    ) : (
                      <span className="section-cover-placeholder">皮</span>
                    )}
                  </div>
                  <div className="section-info">
                    <div className="section-name">{skin.nameZh}</div>
                    <div className="section-en">{skin.nameEn || "官方皮肤"}</div>
                    <div className="section-stat">
                      {secEnabled} 启用 · {secMods.length} Mod
                    </div>
                  </div>
                </div>
              );
            })}

            {/* 自定义分区 */}
            {sections.map((s) => {
              const secMods = group.mods.filter((m) => modSections[m.rel_path] === s.id);
              const secEnabled = secMods.filter((m) => m.enabled).length;
              return (
                <div
                  key={s.id}
                  className={`section-card ${activeSection === s.id ? "active" : ""}`}
                  onClick={() => setActiveSection(s.id)}
                >
                  <div className="section-cover">
                    <span className="section-cover-placeholder">＋</span>
                  </div>
                  <div className="section-info">
                    <div className="section-name">{s.name}</div>
                    <div className="section-en">自定义</div>
                    <div className="section-stat">
                      {secEnabled} 启用 · {secMods.length} Mod
                    </div>
                  </div>
                  <button
                    className="section-del"
                    title="删除此分类"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDeleteSection(s.id);
                    }}
                  >
                    ✕
                  </button>
                </div>
              );
            })}

            {/* 新增分区 */}
            {addingSection ? (
              <div className="section-card adding">
                <input
                  autoFocus
                  value={newSectionName}
                  onChange={(e) => setNewSectionName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleAddSection();
                    if (e.key === "Escape") {
                      setAddingSection(false);
                      setNewSectionName("");
                    }
                  }}
                  onBlur={handleAddSection}
                  placeholder="分类名称"
                  maxLength={20}
                />
              </div>
            ) : (
              <button
                className="section-add"
                onClick={() => {
                  setAddingSection(true);
                  setNewSectionName("");
                }}
              >
                <span className="add-plus">＋</span>
                <span className="add-title">新增分区</span>
                <span className="add-sub">名称与封面自定义</span>
              </button>
            )}
          </div>
        </div>

        {/* 搜索 + 排序 */}
        <div className="detail-toolbar">
          <div className="search-box">
            <span className="search-icon">🔍</span>
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="搜索 Mod 名称…"
            />
            {search && (
              <button className="search-clear" onClick={() => setSearch("")}>
                ✕
              </button>
            )}
          </div>
          <div className="sort-group">
            <span className="sort-label">排序</span>
            {(
              [
                ["name", "名称"],
                ["status", "状态"],
                ["time", "时间"],
                ["custom", "自定义"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                className={`sort-chip ${sortBy === key ? "active" : ""}`}
                onClick={() => setSortBy(key)}
              >
                {label}
              </button>
            ))}
          </div>
          <span className="mod-count-hint">
            {visibleMods.length} / {group.total_count}
          </span>
        </div>

        {viewMode === "list" ? (
          <div className="detail-mod-list">
            {visibleMods.map((m) => (
              <div
                key={m.name}
                className={`detail-mod-row ${m.name === selectedName ? "active" : ""} ${
                  m.enabled ? "on" : "off"
                }`}
                onClick={() => setSelectedName(m.name)}
              >
                <label className="switch" onClick={(e) => e.stopPropagation()}>
                  <input type="checkbox" checked={m.enabled} onChange={() => handleRowToggle(m)} />
                  <span className="slider" />
                </label>
                <div className="row-info">
                  <div className="row-name">{m.name}</div>
                  <div className="row-sub">{m.folder_name}</div>
                </div>
                <button
                  className="row-move"
                  title="移动到其它角色"
                  onClick={(e) => {
                    e.stopPropagation();
                    setMoveTarget("");
                    setMoving(m);
                  }}
                >
                  ⇄
                </button>
                <button
                  className="row-delete"
                  title="删除此 Mod（移到回收站）"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleting(m);
                  }}
                >
                  ✕
                </button>
              </div>
            ))}

            {visibleMods.length === 0 && (
              <div className="empty-hint">
                {search ? `没有匹配「${search}」的 Mod` : "该角色下暂无 Mod"}
              </div>
            )}

            <div className="detail-drop-hint">
              <span className="drop-icon">📁</span>
              <span>点击右上角「导入 Mod」添加，或将压缩包 / Mod 文件夹拖入</span>
            </div>
          </div>
        ) : (
          /* 画廊视图：大图卡片（QAQM 式），点击选中进右栏详情 */
          <div className="detail-mod-list gallery-mode">
            <div className="gallery-grid" style={{ "--gallery-card-w": `${galleryCardSize}px` } as React.CSSProperties}>
              {visibleMods.map((m) => (
                <div
                  key={m.name}
                  className={`gallery-card ${m.enabled ? "enabled" : "disabled"} ${
                    m.name === selectedName ? "selected" : ""
                  }`}
                  title={m.folder_name}
                  onClick={() => setSelectedName(m.name)}
                >
                  <div className="gallery-cover">
                    {m.cover ? (
                      <Cover
                        key={`${m.folder_name}-${coverBust}`}
                        basePath={modsPath}
                        character={group.name}
                        cover={`${m.folder_name}/${m.cover}`}
                        bust={coverBust}
                      />
                    ) : (
                      <span className="gallery-cover-placeholder">🖼️</span>
                    )}
                  </div>
                  <div className="gallery-card-bar">
                    {pinned.includes(`${group.name}/${m.name}`) && (
                      <span className="gallery-pin" title="已置顶">
                        📌
                      </span>
                    )}
                    <span className="gallery-card-name">{m.name}</span>
                    <label className="switch" onClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={m.enabled} onChange={() => handleRowToggle(m)} />
                      <span className="slider" />
                    </label>
                  </div>
                </div>
              ))}
            </div>

            {visibleMods.length === 0 && (
              <div className="empty-hint">
                {search ? `没有匹配「${search}」的 Mod` : "该角色下暂无 Mod"}
              </div>
            )}

            <div className="detail-drop-hint">
              <span className="drop-icon">📁</span>
              <span>点击右上角「导入 Mod」添加，或将压缩包 / Mod 文件夹拖入</span>
            </div>
          </div>
        )}
      </div>

      {/* 右栏：Mod 详情 */}
      <div className="detail-panel">
        {selected ? (
          <>
            <div
              className={`detail-preview ${dragOver ? "drag-over" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setDragOver(true);
              }}
              onDragLeave={() => setDragOver(false)}
              onDrop={(e) => {
                e.preventDefault();
                setDragOver(false);
                const file = e.dataTransfer.files?.[0];
                if (file && file.type.startsWith("image/")) {
                  applyImage(file);
                } else {
                  setSaveMsg("请拖入图片文件（PNG / JPG）");
                }
              }}
            >
              {selected.cover ? (
                <Cover
                  key={`${selected.folder_name}-${coverBust}`}
                  basePath={modsPath}
                  character={group.name}
                  cover={`${selected.folder_name}/${selected.cover}`}
                  bust={coverBust}
                />
              ) : (
                <span className="preview-placeholder">
                  {saving ? "保存中…" : "拖入图片 / Ctrl+V 粘贴设置预览图"}
                </span>
              )}
              <div className="preview-overlay">
                <span>拖入图片 或 Ctrl+V 粘贴</span>
              </div>
            </div>

            {saveMsg && <div className="save-msg">{saveMsg}</div>}

            {/* 标题 + 重命名（QAQM 式） */}
            <div className="detail-title-row">
              {renamingMod ? (
                <input
                  className="detail-title-input"
                  autoFocus
                  value={renameModVal}
                  onChange={(e) => setRenameModVal(e.target.value)}
                  onBlur={handleRenameModCommit}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleRenameModCommit();
                    else if (e.key === "Escape") setRenamingMod(false);
                  }}
                  maxLength={120}
                />
              ) : (
                <>
                  <h3 className="detail-title">{selected.name}</h3>
                  <button
                    className="rename-chip"
                    title="重命名 Mod"
                    onClick={() => {
                      setRenameModVal(selected.name);
                      setRenamingMod(true);
                    }}
                  >
                    ✏️ 重命名
                  </button>
                </>
              )}
            </div>

            {/* 外观分区（下拉，含统计） */}
            <div className="panel-box">
              <div className="panel-box-row">
                <div className="panel-box-head">
                  <span className="panel-box-title">外观分区</span>
                  <span className="panel-box-hint">只调整整理位置，不改变启用状态</span>
                </div>
                <select
                  className="panel-select"
                  value={modSections[selected.rel_path] || "base"}
                  onChange={(e) => handleSetSection(selected, e.target.value)}
                >
                  <option value="base">
                    原角色 · {baseEnabled} 启用 / {baseMods.length} Mod
                  </option>
                  {officialSkins.map((skin) => {
                    const sm = group.mods.filter(
                      (m) => modSections[m.rel_path] === skin.sectionId
                    );
                    return (
                      <option key={skin.sectionId} value={skin.sectionId}>
                        {skin.nameZh} · {sm.filter((m) => m.enabled).length} 启用 / {sm.length} Mod
                      </option>
                    );
                  })}
                  {sections.map((s) => {
                    const sm = group.mods.filter((m) => modSections[m.rel_path] === s.id);
                    return (
                      <option key={s.id} value={s.id}>
                        {s.name} · {sm.filter((m) => m.enabled).length} 启用 / {sm.length} Mod
                      </option>
                    );
                  })}
                </select>
              </div>
            </div>

            {/* 备注 */}
            <div className="panel-box note-box">
              <div className="panel-box-row">
                <div className="panel-box-head">
                  <span className="panel-box-title">📝 备注</span>
                  <button
                    className="panel-link-btn"
                    onClick={() => {
                      setNoteDraft(selNote);
                      setEditingNote(true);
                    }}
                  >
                    {selNote ? "编辑" : "添加"}
                  </button>
                </div>
                {editingNote ? (
                  <div className="note-editor">
                    <textarea
                      className="note-textarea"
                      autoFocus
                      placeholder="点击添加备注..."
                      value={noteDraft}
                      onChange={(e) => setNoteDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) saveNote();
                      }}
                    />
                    <div className="note-editor-actions">
                      <button className="panel-link-btn" onClick={saveNote}>
                        保存
                      </button>
                      <button
                        className="panel-link-btn dim"
                        onClick={() => setEditingNote(false)}
                      >
                        取消
                      </button>
                    </div>
                  </div>
                ) : (
                  <div
                    className={`note-text ${selNote ? "" : "note-empty"}`}
                    onClick={() => {
                      setNoteDraft(selNote);
                      setEditingNote(true);
                    }}
                  >
                    {selNote || "点击添加备注..."}
                  </div>
                )}
              </div>
            </div>

            {/* 状态 */}
            <div className="detail-status">
              <span className={`status-dot ${selected.enabled ? "on" : "off"}`} />
              {selected.enabled ? "已启用" : "已停用"}
            </div>

            {/* 快捷键（解析 mod.ini [Key*] 段，可直接改键） */}
            <div className="panel-box hk-box">
              <div className="panel-box-row">
                <div className="panel-box-head">
                  <span className="panel-box-title">⌨ 快捷键</span>
                  <button
                    className="panel-icon-btn"
                    title="重新解析快捷键"
                    onClick={() => setCoverBust(Date.now())}
                  >
                    🔄
                  </button>
                </div>
                {hkEditing && (
                  <div className="hk-capture-bar">
                    {hkCaptured ? (
                      <span className="keycap preview">{hkCaptured}</span>
                    ) : (
                      <span className="hk-capture-hint">请按下新的按键组合…（Esc 取消 / Enter 确认）</span>
                    )}
                    <button
                      className="panel-link-btn"
                      onClick={saveHotkey}
                      disabled={!hkCaptured}
                    >
                      确认
                    </button>
                    <button
                      className="panel-link-btn dim"
                      onClick={() => {
                        setHkEditing(null);
                        setHkCaptured("");
                      }}
                    >
                      取消
                    </button>
                  </div>
                )}
                <div className="hk-list">
                  {hotkeys && hotkeys.length > 0 ? (
                    hotkeys.map((hk) => (
                      <div key={hk.section} className="hk-row">
                        <span className="hk-name" title={`[${hk.section}] ${hk.raw}`}>
                          {hk.description}
                        </span>
                        {hkEditing?.section === hk.section ? (
                          <span className="keycap editing">编辑中…</span>
                        ) : (
                          <span className="hk-right">
                            <span className="keycap">{hk.keys.join("+") || hk.raw}</span>
                            <button
                              className="hk-pen"
                              title="修改此快捷键"
                              onClick={() => {
                                setHkEditing({ section: hk.section, desc: hk.description });
                                setHkCaptured("");
                              }}
                            >
                              ✏️
                            </button>
                          </span>
                        )}
                      </div>
                    ))
                  ) : (
                    <div className="hk-empty">
                      {hotkeys ? "该 Mod 没有定义快捷键（无 [Key*] 段）" : "解析中…"}
                    </div>
                  )}
                </div>
              </div>
            </div>

            {/* 操作按钮行（QAQM 式） */}
            <div className="panel-actions">
              <button
                className={selPinned ? "pin-btn active" : "pin-btn"}
                onClick={handleTogglePin}
                title={selPinned ? "取消置顶" : "置顶此 Mod（排序时优先显示）"}
              >
                {selPinned ? "📌 取消置顶" : "🔺 置顶"}
              </button>
              <button className="panel-btn" onClick={() => handleOpenFolder(selected)}>
                📁 源文件
              </button>
              <button className="panel-btn repair" disabled title="功能开发中（待办 3）">
                🛠 修复
              </button>
              <button
                className="panel-btn delete"
                onClick={() => setDeleting(selected)}
                title="删除此 Mod（移到回收站）"
              >
                🗑 删除
              </button>
            </div>
          </>
        ) : (
          <div className="detail-empty">选择左侧的 Mod 查看详情</div>
        )}
      </div>

      {/* 删除确认弹窗（二次确认） */}
      {moving && (
        <div className="modal-overlay" onClick={() => setMoving(null)}>
          <div className="confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="confirm-icon">⇄</div>
            <h3 className="confirm-title">移动 Mod</h3>
            <p className="confirm-text">
              把 "<strong>{moving.name}</strong>" 从{" "}
              <strong>{moving.character}</strong> 移动到：
            </p>
            <div className="move-char-list">
              {allCharacterNames
                .filter((n) => n !== moving.character)
                .map((n) => (
                  <button
                    key={n}
                    className={`move-char-item ${moveTarget === n ? "active" : ""}`}
                    onClick={() => setMoveTarget(n)}
                  >
                    {n}
                  </button>
                ))}
            </div>
            <p className="confirm-warn">
              移动后保持当前的启用/禁用状态，预设与更新检测记录会同步更新。
            </p>
            <div className="confirm-actions">
              <button
                className="confirm-primary"
                disabled={!moveTarget || moveBusy}
                onClick={handleMoveMod}
              >
                {moveBusy ? "移动中…" : "确认移动"}
              </button>
              <button className="confirm-cancel" onClick={() => setMoving(null)}>
                取消
              </button>
            </div>
          </div>
        </div>
      )}
      {deleting && (
        <div className="modal-overlay" onClick={() => setDeleting(null)}>
          <div className="confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="confirm-icon">⚠️</div>
            <h3 className="confirm-title">确认删除</h3>
            <p className="confirm-text">
              确定要把模组 "<strong>{deleting.name}</strong>" 移入回收站吗？
              <br />
              之后可在系统回收站中恢复。
            </p>
            {deleting.enabled && (
              <p className="confirm-warn">此模组当前处于启用状态</p>
            )}
            <div className="confirm-actions">
              <button className="confirm-primary" onClick={handleConfirmDelete}>
                移入回收站
              </button>
              <button className="confirm-cancel" onClick={() => setDeleting(null)}>
                取消
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ===== 预设视图 =====
// ===== GameBanana 市场类型 =====
type MarketCategory = { id: number; name: string; item_count: number; icon_url: string; group: string };
type MarketTopSub = {
  id: number;
  name: string;
  url: string;
  image: string;
  author: string;
  likes: number;
  period: string;
};
type MarketComment = {
  id: number;
  author: string;
  avatar: string;
  text: string;
  date_added: number;
};
type MarketMod = {
  id: number;
  name: string;
  url: string;
  author: string;
  image: string;
  likes: number;
  views: number;
  date_added: number;
};
type MarketModList = { records: MarketMod[]; total: number; page: number; has_more: boolean };
type MarketFile = { id: number; filename: string; size: number; download_url: string };
type MarketModDetail = {
  id: number;
  name: string;
  url: string;
  author: string;
  text: string;
  images: string[];
  files: MarketFile[];
  likes: number;
  views: number;
  downloads: number;
  category: string;
};

const MARKET_SORTS: { value: string; label: string }[] = [
  { value: "", label: "默认排序" },
  { value: "Generic_MostLiked", label: "最多点赞" },
  { value: "Generic_MostDownloaded", label: "最多下载" },
  { value: "Generic_MostViewed", label: "最多浏览" },
];

/// 归一化名字：去空格/标点/括号，便于本地角色与 GameBanana 分类互相匹配
function normName(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]/g, "");
}

/// GameBanana 分类英文名 ↔ 本地中文角色目录名（来源：终末地官网干员情报）
const GB_CN_NAME_MAP: [string, string][] = [
  ["Si", "祀"],
  ["Arcane", "诀"],
  ["Endministrator (F)", "女主"],
  ["Endministrator (M)", "男主"],
  ["Perlica", "佩丽卡"],
  ["Chen Qianyu", "陈千语"],
  ["Typhoeus", "提弗洛斯"],
  ["Purrchena", "噗切娜"],
  ["Liino", "梨诺"],
  ["Mi Fu", "弭弗"],
  ["Camille", "卡繆"],
  ["Zhuang Fangyi", "庄方宜"],
  ["Tangtang", "汤汤"],
  ["Rossi", "洛茜"],
  ["Laevatain", "莱万汀"],
  ["Yvonne", "伊冯"],
  ["Gilberta", "洁尔佩塔"],
  ["Ardelia", "艾尔黛拉"],
  ["Ember", "余烬"],
  ["Last Rite", "别礼"],
  ["Lifeng", "黎风"],
  ["Pogranichnik", "骏卫"],
  ["Alesh", "阿列什"],
  ["Arclight", "弧光"],
  ["Avywenna", "艾维文娜"],
  ["Da Pan", "大潘"],
  ["Snowshine", "昼雪"],
  ["Wulfgard", "狼卫"],
  ["Xaihi", "赛希"],
  ["Akekuri", "秋栗"],
  ["Antal", "安塔尔"],
  ["Catcher", "卡契尔"],
  ["Estella", "埃特拉"],
  ["Fluorite", "萤石"],
];

function gbNameToCn(gbName: string): string | undefined {
  const n = gbName.toLowerCase().trim();
  return GB_CN_NAME_MAP.find(([gb]) => gb.toLowerCase() === n)?.[1];
}
function cnToGbName(cn: string): string | undefined {
  return GB_CN_NAME_MAP.find(([, c]) => c === cn.trim())?.[0];
}

/// 两个名字是否匹配（归一化后相等或互为前缀）
function matchPair(a: string, b: string): boolean {
  const x = normName(a);
  const y = normName(b);
  if (!x || !y) return false;
  return x === y || x.startsWith(y) || y.startsWith(x);
}

/// 用本地角色名（映射表/拼音）匹配 GameBanana 分类
function matchCategory(
  cats: MarketCategory[],
  charName: string,
  groups: CharacterGroup[]
): MarketCategory | undefined {
  const gb = cnToGbName(charName);
  if (gb) {
    const hit = cats.find((c) => c.name.toLowerCase() === gb.toLowerCase());
    if (hit) return hit;
  }
  const g = groups.find((x) => x.name === charName);
  if (g) {
    const hit = cats.find(
      (c) => matchPair(c.name, g.name) || (g.pinyin && matchPair(c.name, g.pinyin))
    );
    if (hit) return hit;
  }
  return cats.find((c) => matchPair(c.name, charName));
}

/// 用 GameBanana 分类名匹配本地角色目录名（映射表 → 拼音 → 名称前缀）
function matchLocalGroup(
  groups: CharacterGroup[],
  gbName: string
): CharacterGroup | undefined {
  const cn = gbNameToCn(gbName);
  if (cn) {
    const g = groups.find((x) => x.name === cn);
    if (g) return g;
  }
  return groups.find(
    (g) => matchPair(gbName, g.name) || (g.pinyin && matchPair(gbName, g.pinyin))
  );
}

/// 粗略清洗 GameBanana 的描述 HTML（去脚本/样式/内联事件）
function sanitizeGbHtml(html: string): string {
  return html
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<iframe[\s\S]*?<\/iframe>/gi, "")
    .replace(/<style[\s\S]*?<\/style>/gi, "")
    .replace(/\son\w+\s*=\s*"[^"]*"/gi, "")
    .replace(/\son\w+\s*=\s*'[^']*'/gi, "");
}

function fmtCompact(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1).replace(/\.0$/, "") + "k";
  return String(n);
}

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return (bytes / 1024 / 1024).toFixed(1) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(0) + " KB";
  return bytes + " B";
}

// ===== 全局下载中心（右下角浮窗，不持久化，退出程序即消失） =====
function DlCenter({
  tasks,
  minimized,
  onToggle,
  onCancel,
  onRemove,
}: {
  tasks: DlTask[];
  minimized: boolean;
  onToggle: () => void;
  onCancel: (taskId: string) => void;
  onRemove: (taskId: string) => void;
}) {
  if (minimized) {
    return (
      <button
        className="dl-fab"
        onClick={onToggle}
        title={`下载列表（${tasks.length} 个任务）`}
      >
        ⬇
        {tasks.length > 0 && <span className="dl-fab-badge">{tasks.length}</span>}
      </button>
    );
  }
  return (
    <div className="dl-center">
      <div className="dl-center-head">
        <span>⬇ 下载列表</span>
        <button className="dl-center-min" onClick={onToggle} title="最小化">
          —
        </button>
      </div>
      <div className="dl-center-body">
        {tasks.length === 0 ? (
          <div className="dl-center-empty">暂无下载任务</div>
        ) : (
          tasks.map((t) => {
            if (t.status !== "active") {
              // 终态：完成/失败/取消，保留显示，✕ = 从列表移除
              const done = t.status === "done";
              return (
                <div key={t.id} className={`dl-task ${done ? "dl-task-done" : "dl-task-failed"}`}>
                  <div className="dl-task-name" title={t.modUrl ? `打开 GameBanana 页面：${t.filename}` : t.filename}>
                    {t.modUrl ? (
                      <span className="dl-task-link" onClick={() => openUrl(t.modUrl!)}>
                        {done ? "✓ " : t.status === "failed" ? "✗ " : ""}
                        {t.filename}
                      </span>
                    ) : (
                      <>
                        {done ? "✓ " : t.status === "failed" ? "✗ " : ""}
                        {t.filename}
                      </>
                    )}
                  </div>
                  <div className="dl-task-row">
                    {done && t.targetPath ? (
                      <span
                        className={`dl-task-info ok dl-task-open`}
                        title={`打开所在目录：${t.targetPath}`}
                        onClick={() => revealItemInDir(t.targetPath!)}
                      >
                        已完成 · {t.note}
                      </span>
                    ) : (
                      <span className={`dl-task-info ${done ? "ok" : "bad"}`}>
                        {done ? "已完成" : t.status === "failed" ? "下载失败" : "已取消"}
                        {t.note ? ` · ${t.note}` : ""}
                      </span>
                    )}
                    <button
                      className="dl-task-cancel"
                      title="从列表移除"
                      onClick={() => onRemove(t.id)}
                    >
                      ✕
                    </button>
                  </div>
                </div>
              );
            }
            // 进行中：进度条 + 取消
            const pct = t.total ? Math.min(100, (t.downloaded / t.total) * 100) : null;
            return (
              <div key={t.id} className="dl-task">
                <div className="dl-task-name" title={t.modUrl ? `打开 GameBanana 页面：${t.filename}` : t.filename}>
                  {t.modUrl ? (
                    <span className="dl-task-link" onClick={() => openUrl(t.modUrl!)}>
                      {t.filename}
                    </span>
                  ) : (
                    t.filename
                  )}
                </div>
                <div className="dl-task-bar">
                  <div
                    className="dl-task-bar-fill"
                    style={{ width: pct !== null ? `${pct}%` : "40%" }}
                  />
                </div>
                <div className="dl-task-row">
                  <span className="dl-task-info">
                    {pct !== null
                      ? `${Math.round(pct)}% · ${fmtSize(t.downloaded)} / ${fmtSize(t.total ?? 0)}`
                      : `${fmtSize(t.downloaded)}`}
                  </span>
                  <button className="dl-task-cancel" title="取消下载" onClick={() => onCancel(t.id)}>
                    ✕
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

// ===== Mod 市场（GameBanana 在线检索，复刻 IMM 在线模式） =====
/** 游戏 id -> 显示名（市场副标题用） */
const GAME_NAMES: Record<string, string> = {
  endfield: "明日方舟：终末地",
  starrail: "崩坏：星穹铁道",
  wuthering: "鸣潮",
  zenless: "绝区零",
};
/** 全局下载中心的一个下载任务（终态保留在列表里，退出程序即清空） */
type DlTaskStatus = "active" | "done" | "failed" | "cancelled";
type DlTask = {
  id: string;
  filename: string;
  downloaded: number;
  total: number | null;
  status: DlTaskStatus;
  /** 终态说明：已完成（安装/保存路径）、失败原因等 */
  note?: string;
  /** GameBanana 页面链接（任务名可点击跳转） */
  modUrl?: string;
  /** 完成后的目标路径（安装目录 / 下载的文件，绿色字可点击在资源管理器打开） */
  targetPath?: string;
};
function MarketView({
  gameId,
  groups,
  modsPath,
  initialChar,
  heroScale,
  cardSize,
  onInstalled,
  setMsg,
  setError,
  dlTasks,
  onTaskEnd,
  onCancelTask,
  onRegisterUrl,
}: {
  /** 当前激活游戏 id（市场随游戏切换；key 绑定此值使切换时重建） */
  gameId: string;
  groups: CharacterGroup[];
  modsPath: string;
  initialChar: string;
  /** 中央推荐大卡片的显示大小（占容器宽度百分比，等比缩放） */
  heroScale: number;
  /** 小卡片宽度 px（系统设置可调） */
  cardSize: number;
  onInstalled: () => void;
  setMsg: (s: string) => void;
  setError: (s: string) => void;
  /** 进行中的下载任务（全局下载中心数据源） */
  dlTasks: DlTask[];
  /** 任务结束（成功/失败/取消）：转为终态保留在列表；targetPath 供「打开所在目录」 */
  onTaskEnd: (
    taskId: string,
    status: "done" | "failed" | "cancelled",
    note?: string,
    targetPath?: string,
  ) => void;
  /** 取消下载任务 */
  onCancelTask: (taskId: string) => void;
  /** 触发下载时登记 GameBanana 页面链接 */
  onRegisterUrl: (taskId: string, modUrl: string) => void;
}) {
  const [categories, setCategories] = useState<MarketCategory[]>([]);
  const [catId, setCatId] = useState(0); // 0 = 最新发布（首页订阅流）
  const [sort, setSort] = useState("");
  const [page, setPage] = useState(1);
  const [termInput, setTermInput] = useState("");
  const [term, setTerm] = useState(""); // 回车提交后生效
  const [list, setList] = useState<MarketModList | null>(null);
  const [loading, setLoading] = useState(false);
  const [detail, setDetail] = useState<MarketModDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [shot, setShot] = useState(0); // 详情面板当前截图下标
  // 下载按钮的二级菜单（当前展开的文件 id；null = 关闭）
  const [dlMenu, setDlMenu] = useState<number | null>(null);
  // IMM 复刻：首页 TopSubs 轮播
  const [topSubs, setTopSubs] = useState<MarketTopSub[]>([]);
  const [topSubIndex, setTopSubIndex] = useState(0);
  // 手动翻页计数：变化时重置自动播放计时，避免刚点完立刻被自动翻走
  const [subNavKey, setSubNavKey] = useState(0);
  // IMM 复刻：详情评论区
  const [comments, setComments] = useState<MarketComment[]>([]);
  const [commentsLoading, setCommentsLoading] = useState(false);
  // 刷新令牌：递增时重新拉取分类 / 轮播 / 列表
  const [refreshKey, setRefreshKey] = useState(0);
  // 初始角色匹配只在首次加载做，避免刷新时覆盖用户手动选的分类
  const initialMatchedRef = useRef(false);

  // 加载分类（随刷新更新）；初始角色匹配只做一次
  useEffect(() => {
    invoke<MarketCategory[]>("market_categories")
      .then((cats) => {
        setCategories(cats);
        if (initialChar && !initialMatchedRef.current) {
          initialMatchedRef.current = true;
          const m = matchCategory(cats, initialChar, groups);
          if (m) setCatId(m.id);
        }
      })
      .catch((e) => setError(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  // 首页轮播推荐（随刷新更新）
  useEffect(() => {
    invoke<MarketTopSub[]>("market_top_subs")
      .then((subs) => setTopSubs(subs.slice(0, 12)))
      .catch(() => setTopSubs([])); // 轮播失败不影响主列表
  }, [refreshKey]);

  // 轮播自动播放（5 秒，与 IMM 一致；手动翻页后重新计时）
  useEffect(() => {
    if (topSubs.length <= 1) return;
    const t = window.setInterval(() => {
      setTopSubIndex((i) => (i + 1) % topSubs.length);
    }, 5000);
    return () => window.clearInterval(t);
  }, [topSubs.length, subNavKey]);

  // 左右翻页（循环）
  const goSub = (delta: number) => {
    if (topSubs.length <= 1) return;
    setTopSubIndex((i) => (i + delta + topSubs.length) % topSubs.length);
    setSubNavKey((k) => k + 1);
  };

  // ===== 拖拽轮播 =====
  const dragRef = useRef<{ x: number; y: number; active: boolean; lastDx: number } | null>(null);
  const dragMovedRef = useRef(false);
  const [dragX, setDragX] = useState(0);

  const onSlidePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (topSubs.length <= 1) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    dragMovedRef.current = false;
    dragRef.current = { x: e.clientX, y: e.clientY, active: false, lastDx: 0 };
  };

  const onSlidePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.active) {
      if (Math.abs(dx) < 6) return;
      // 纵向手势不拦截，交还给列表滚动
      if (Math.abs(dy) > Math.abs(dx)) {
        dragRef.current = null;
        return;
      }
      d.active = true;
      e.currentTarget.setPointerCapture(e.pointerId);
    }
    // 阻尼：拖得越远增益越小，上限 140px
    const damped = Math.sign(dx) * Math.min(140, Math.abs(dx) * 0.6);
    d.lastDx = damped;
    dragMovedRef.current = true;
    setDragX(damped);
  };

  const endSlideDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d) return;
    if (d.active) {
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        /* 指针捕获已自动释放 */
      }
      // 拖动超过 60px 视为翻页
      if (Math.abs(d.lastDx) >= 60) goSub(d.lastDx < 0 ? 1 : -1);
    }
    setDragX(0);
  };

  const onSlideClick = () => {
    if (dragMovedRef.current) {
      dragMovedRef.current = false;
      return; // 刚拖过，不当作点击
    }
    if (topSubs[topSubIndex]) openDetail(topSubs[topSubIndex].id);
  };

  // 拉取列表
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    (async () => {
      try {
        const r = term
          ? await invoke<MarketModList>("market_search", { term, page })
          : await invoke<MarketModList>("market_mods", { categoryId: catId, sort, page });
        if (!cancelled) setList(r);
      } catch (e) {
        if (!cancelled) {
          setList({ records: [], total: 0, page: 1, has_more: false });
          setError(String(e));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [catId, sort, page, term, refreshKey]);

  const detailRef = useRef<MarketModDetail | null>(null);
  detailRef.current = detail;

  // 选分类 / 排序时回到第 1 页
  function handleCatChange(id: number) {
    setCatId(id);
    setPage(1);
    setTerm("");
    setTermInput("");
  }
  function handleSortChange(s: string) {
    setSort(s);
    setPage(1);
  }
  function handleSearchSubmit() {
    const t = termInput.trim();
    if (t) {
      setTerm(t);
      setCatId(0); // 搜索覆盖分类过滤
    } else {
      setTerm("");
    }
    setPage(1);
  }

  // 刷新：重新拉取分类、轮播和当前列表（保持当前分类/搜索/排序/页码）
  function handleRefresh() {
    setRefreshKey((k) => k + 1);
  }

  async function openDetail(id: number) {
    setDetailLoading(true);
    setShot(0);
    setComments([]);
    try {
      const d = await invoke<MarketModDetail>("market_mod_detail", { modId: id });
      setDetail(d);
      // 评论按需加载，失败不影响详情展示
      setCommentsLoading(true);
      invoke<MarketComment[]>("market_comments", { modId: id })
        .then(setComments)
        .catch(() => setComments([]))
        .finally(() => setCommentsLoading(false));
    } catch (e) {
      setError(String(e));
    } finally {
      setDetailLoading(false);
    }
  }

  /// 安装目标角色目录：优先匹配本地角色名，否则用分类名
  function targetCharacter(d: MarketModDetail): string {
    const gbCat = categories.find((c) => c.id === catId)?.name || d.category;
    const local = matchLocalGroup(groups, gbCat);
    return local ? local.name : gbCat;
  }

  async function handleDownload(f: MarketFile, d: MarketModDetail) {
    const char = targetCharacter(d);
    const taskId = String(f.id);
    onRegisterUrl(taskId, d.url);
    try {
      const installed = await invoke<string>("market_download", {
        modsPath,
        downloadUrl: f.download_url,
        filename: f.filename,
        character: char,
        modName: d.name,
        // 安装时把下载页预览图一并存为 mod 封面
        coverUrl: d.images[0] || "",
        taskId,
        // 记录安装来源（更新检测用）
        gbModId: d.id,
        pageUrl: d.url,
      });
      setMsg(`已安装到 ${installed}`);
      onInstalled();
      onTaskEnd(
        taskId,
        "done",
        `已安装到 ${installed}`,
        `${modsPath}/${char}/${installed}`,
      );
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已取消")) {
        setMsg("已取消下载");
        onTaskEnd(taskId, "cancelled");
      } else {
        setError(msg);
        onTaskEnd(taskId, "failed", msg);
      }
    }
  }

  /// 仅下载：保存到用户设置的下载文件夹（默认系统下载文件夹），不安装
  async function handleSaveDownload(f: MarketFile, d: MarketModDetail) {
    const taskId = `save-${f.id}`;
    onRegisterUrl(taskId, d.url);
    try {
      const dir = await invoke<string>("get_download_dir");
      const path = await invoke<string>("market_download_file", {
        downloadUrl: f.download_url,
        filename: f.filename,
        saveDir: dir,
        taskId,
      });
      setMsg(`已下载到 ${path}`);
      onTaskEnd(taskId, "done", `已保存到 ${path}`, path);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已取消")) {
        setMsg("已取消下载");
        onTaskEnd(taskId, "cancelled");
      } else {
        setError(msg);
        onTaskEnd(taskId, "failed", msg);
      }
    }
  }

  const activeCat = categories.find((c) => c.id === catId);
  const totalPages = list ? Math.max(1, Math.ceil(list.total / (term ? 15 : 24))) : 1;

  return (
    <div className="market-view">
      {/* 顶栏：标题 + 分类选择 + 搜索 + 排序 */}
      <div className="market-header panel-box">
        <div className="char-header-top">
          <h2>🛒 Mod 市场</h2>
          <span className="char-subtitle">
            来自 GameBanana 的 {GAME_NAMES[gameId] || gameId} Mod 在线检索
          </span>
          <div className="char-header-actions">
            <button
              className="panel-btn"
              onClick={handleRefresh}
              disabled={loading}
              title="重新拉取分类、推荐和当前列表"
            >
              <span
                className={loading ? "market-refresh-icon spinning" : "market-refresh-icon"}
              >
                🔄
              </span>
              {loading ? "刷新中…" : "刷新"}
            </button>
          </div>
        </div>
        <div className="char-toolbar market-toolbar">
          <select
            className="market-cat-select"
            value={catId}
            onChange={(e) => handleCatChange(Number(e.target.value))}
            title="选择角色分类"
          >
            <option value={0}>🔥 最新发布（全部分类）</option>
            {categories.filter((c) => c.group !== "general").map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}（{c.item_count}）
              </option>
            ))}
            {categories.some((c) => c.group === "general") && (
              <optgroup label="武器 / 其他">
                {categories.filter((c) => c.group === "general").map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}（{c.item_count}）
                  </option>
                ))}
              </optgroup>
            )}
          </select>
          <div className="market-search-box">
            <input
              value={termInput}
              placeholder="搜索 GameBanana Mod 名称..."
              onChange={(e) => setTermInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleSearchSubmit();
              }}
            />
            <button
              className="market-search-btn"
              onClick={handleSearchSubmit}
              title="搜索"
            >
              🔍
            </button>
            {term && (
              <button
                className="market-search-clear"
                title="清除搜索"
                onClick={() => {
                  setTerm("");
                  setTermInput("");
                  setPage(1);
                }}
              >
                ✕
              </button>
            )}
          </div>
          <select
            className="market-sort-select"
            value={sort}
            onChange={(e) => handleSortChange(e.target.value)}
            title="排序方式"
          >
            {MARKET_SORTS.map((s) => (
              <option key={s.value} value={s.value}>
                {s.label}
              </option>
            ))}
          </select>
        </div>
        <div className="market-status">
          {term ? (
            <span>
              搜索「{term}」共 {list?.total ?? 0} 个结果
            </span>
          ) : (
            <span>
              {catId === 0
                ? "最新发布的 Mod"
                : `${activeCat?.name ?? ""} 分类共 ${list?.total ?? 0} 个 Mod`}
            </span>
          )}
        </div>
      </div>

      {/* Mod 卡片网格 */}
      <div className="market-body">
        {/* IMM 复刻：首页 TopSubs 轮播推荐 */}
        {catId === 0 && !term && topSubs.length > 0 && (
          <div
            className="market-carousel"
            style={{ "--market-hero-w": `${heroScale}%` } as React.CSSProperties}
          >
            <div
              className={`market-carousel-slide${dragX !== 0 ? " dragging" : ""}`}
              style={{
                backgroundImage: topSubs[topSubIndex]?.image ? `url(${topSubs[topSubIndex].image})` : undefined,
                transform: dragX ? `translateX(${dragX}px)` : undefined,
              }}
              onClick={onSlideClick}
              onPointerDown={onSlidePointerDown}
              onPointerMove={onSlidePointerMove}
              onPointerUp={endSlideDrag}
              onPointerCancel={endSlideDrag}
              onDragStart={(e) => e.preventDefault()}
              title={topSubs[topSubIndex]?.name}
            >
              <div className="market-carousel-caption">
                <span className="market-carousel-title">{topSubs[topSubIndex]?.name}</span>
                <span className="market-carousel-author">by {topSubs[topSubIndex]?.author || "?"}</span>
              </div>
            </div>
            {topSubs.length > 1 && (
              <>
                <button
                  className="market-carousel-nav prev"
                  title="上一个"
                  aria-label="上一个"
                  onClick={() => goSub(-1)}
                >
                  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M15 18l-6-6 6-6" />
                  </svg>
                </button>
                <button
                  className="market-carousel-nav next"
                  title="下一个"
                  aria-label="下一个"
                  onClick={() => goSub(1)}
                >
                  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M9 18l6-6-6-6" />
                  </svg>
                </button>
              </>
            )}
            {topSubs.length > 1 && (
              <div className="market-carousel-dots">
                {topSubs.map((_, i) => (
                  <button
                    key={i}
                    className={i === topSubIndex ? "active" : ""}
                    title={`第 ${i + 1} 张`}
                    onClick={() => {
                      setTopSubIndex(i);
                      setSubNavKey((k) => k + 1);
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}
        {loading ? (
          <div className="market-loading">加载中...</div>
        ) : !list || list.records.length === 0 ? (
          <div className="market-empty">
            <div className="market-empty-icon">🕸️</div>
            <div>没有找到 Mod</div>
            <div className="hint">换个分类或搜索词试试</div>
          </div>
        ) : (
          <div className="market-grid" style={{ "--market-card-w": `${cardSize}px` } as React.CSSProperties}>
            {list.records.map((m) => (
              <div key={m.id} className="market-card" onClick={() => openDetail(m.id)}>
                <div className="market-card-cover">
                  {m.image ? (
                    <img src={m.image} loading="lazy" alt={m.name} />
                  ) : (
                    <div className="market-card-noimg">🧩</div>
                  )}
                </div>
                <div className="market-card-info">
                  <div className="market-card-name" title={m.name}>
                    {m.name}
                  </div>
                  <div className="market-card-meta">
                    <span className="market-card-author" title={m.author}>
                      {m.author || "?"}
                    </span>
                    <span className="market-card-stats">
                      <span title="点赞">👍 {fmtCompact(m.likes)}</span>
                      <span title="浏览">👁 {fmtCompact(m.views)}</span>
                    </span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        {/* 分页 */}
        {!loading && list && list.records.length > 0 && (
          <div className="market-pager">
            <button
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
            >
              ← 上一页
            </button>
            <span>
              第 {page} / {Math.max(totalPages, page)} 页
            </span>
            <button
              disabled={!list.has_more && page >= totalPages}
              onClick={() => setPage((p) => p + 1)}
            >
              下一页 →
            </button>
          </div>
        )}
      </div>

      {/* 详情浮层 */}
      {detailLoading && <div className="market-detail-loading">加载详情中...</div>}
      {detail && (
        <div
          className="market-detail-mask"
          onClick={() => {
            setDetail(null);
            setDlMenu(null);
          }}
        >
          <div className="market-detail" onClick={(e) => e.stopPropagation()}>
            <button className="market-detail-close" onClick={() => { setDetail(null); setDlMenu(null); }} title="关闭">
              ✕
            </button>
            <div className="market-detail-head">
              <h3>{detail.name}</h3>
              {detail.url && (
                <button
                  className="market-detail-link"
                  onClick={() => openUrl(detail.url)}
                  title={`在浏览器打开 GameBanana 页面`}
                >
                  🔗 GameBanana 页面
                </button>
              )}
              <div className="market-detail-meta">
                <span>👤 {detail.author}</span>
                <span>👍 {fmtCompact(detail.likes)}</span>
                <span>👁 {fmtCompact(detail.views)}</span>
                <span>⬇ {fmtCompact(detail.downloads)}</span>
                {detail.category && (
                  <span className="market-detail-cat">{detail.category}</span>
                )}
              </div>
            </div>
            {detail.images.length > 0 && (
              <div className="market-detail-shots">
                <img src={detail.images[shot]} alt={detail.name} />
                {detail.images.length > 1 && (
                  <div className="market-detail-thumbs">
                    {detail.images.map((img, i) => (
                      <img
                        key={i}
                        src={img}
                        className={i === shot ? "active" : ""}
                        onClick={() => setShot(i)}
                        alt=""
                      />
                    ))}
                  </div>
                )}
              </div>
            )}
            <div className="market-detail-files">
              {detail.files.map((f) => {
                const p = dlTasks.find((x) => x.id === String(f.id));
                const pct = p && p.total ? Math.min(100, (p.downloaded / p.total) * 100) : null;
                return (
                  <div key={f.id} className="market-file-row">
                    <span className="market-file-name" title={f.filename}>
                      📦 {f.filename}
                    </span>
                    <span className="market-file-size">{fmtSize(f.size)}</span>
                    {p ? (
                      <div className="market-dl-progress">
                        <div
                          className="market-dl-bar"
                          style={{ width: pct !== null ? `${pct}%` : "40%" }}
                        />
                        <span className="market-dl-text">
                          {pct !== null
                            ? `${Math.round(pct)}%`
                            : `${fmtSize(p.downloaded)}`}
                        </span>
                        <button
                          className="market-dl-cancel"
                          title="取消下载"
                          onClick={() => onCancelTask(String(f.id))}
                        >
                          ✕
                        </button>
                      </div>
                    ) : (
                      <div className="market-dl-wrap">
                        <button
                          className="market-dl-btn"
                          onClick={() => setDlMenu(dlMenu === f.id ? null : f.id)}
                          title="选择下载方式"
                        >
                          ⬇ 下载 ▾
                        </button>
                        {dlMenu === f.id && (
                          <div className="market-dl-menu">
                            <button
                              className="market-dl-menu-item"
                              disabled={!modsPath}
                              title={modsPath ? "下载并安装到对应角色目录" : "尚未配置 Mods 目录"}
                              onClick={() => {
                                setDlMenu(null);
                                handleDownload(f, detail);
                              }}
                            >
                              📦 下载并安装
                            </button>
                            <button
                              className="market-dl-menu-item"
                              title="保存到下载文件夹（系统设置里可更改）"
                              onClick={() => {
                                setDlMenu(null);
                                handleSaveDownload(f, detail);
                              }}
                            >
                              💾 仅下载
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
              {detail.files.length === 0 && <span className="hint">该 Mod 没有可下载的文件</span>}
            </div>
            <div
              className="market-detail-desc"
              dangerouslySetInnerHTML={{ __html: sanitizeGbHtml(detail.text) }}
            />
            {/* IMM 复刻：评论区 */}
            <div className="market-comments">
              <div className="market-comments-title">💬 评论 {comments.length > 0 && `(${comments.length})`}</div>
              {commentsLoading ? (
                <div className="hint">评论加载中...</div>
              ) : comments.length > 0 ? (
                comments.map((c) => (
                  <div key={c.id} className="market-comment">
                    {c.avatar && (
                      <img className="market-comment-avatar" src={c.avatar} alt="" loading="lazy" />
                    )}
                    <div className="market-comment-body">
                      <div className="market-comment-meta">
                        <span className="market-comment-author">{c.author || "匿名"}</span>
                        <span className="market-comment-date">
                          {new Date(c.date_added * 1000).toLocaleDateString()}
                        </span>
                      </div>
                      <div
                        className="market-comment-text"
                        dangerouslySetInnerHTML={{ __html: sanitizeGbHtml(c.text) }}
                      />
                    </div>
                  </div>
                ))
              ) : (
                <div className="hint">暂无评论</div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function PresetsView({
  presets,
  presetName,
  setPresetName,
  onSave,
  onApply,
  onDelete,
}: {
  presets: Preset[];
  presetName: string;
  setPresetName: (v: string) => void;
  onSave: () => void;
  onApply: (name: string) => void;
  onDelete: (name: string) => void;
}) {
  return (
    <div className="view">
      <div className="view-header">
        <div>
          <h2>预设配置</h2>
          <div className="subtitle">
            保存当前启用的 Mod 组合，一键切换。部件的切换状态由 Persist Bridge 自动保存，不包含在预设内。
          </div>
        </div>
      </div>
      <div className="preset-form">
        <input value={presetName} onChange={(e) => setPresetName(e.target.value)} placeholder="预设名称（如：日常、战斗、清凉）" />
        <button className="primary" onClick={onSave}>💾 保存当前方案</button>
      </div>
      <div className="preset-list">
        {presets.map((p) => (
          <div key={p.name} className="preset-card">
            <div className="preset-info" onClick={() => onApply(p.name)}>
              <div className="preset-title">{p.name}</div>
              <div className="preset-sub">{p.enabled_mods.length} 个 Mod · {p.created_at}</div>
            </div>
            <div className="preset-actions">
              <button className="primary" onClick={() => onApply(p.name)}>应用</button>
              <button className="danger" onClick={() => onDelete(p.name)}>删除</button>
            </div>
          </div>
        ))}
        {presets.length === 0 && <div className="empty-hint">暂无预设，保存后可一键切换 Mod 方案</div>}
      </div>
    </div>
  );
}

// ===== 切换游戏视图 =====
function GamesView({
  games,
  activeGameId,
  onSwitch,
  onAddGame,
  onRemoveGame,
  onRefreshGames,
}: {
  games: Game[];
  activeGameId: string;
  onSwitch: (id: string) => void;
  onAddGame: () => void;
  onRemoveGame: (id: string) => void;
  onRefreshGames: () => void;
}) {
  const [builtins, setBuiltins] = useState<BuiltinGameInfo[]>([]);
  const [editingId, setEditingId] = useState("");
  const [editValue, setEditValue] = useState("");
  const [covers, setCovers] = useState<Record<string, string>>({});
  const [bust, setBust] = useState(0);
  // GameBanana 市场绑定弹窗
  const [bindingGame, setBindingGame] = useState<Game | null>(null);
  const [bindGameId, setBindGameId] = useState("");
  const [bindCatId, setBindCatId] = useState("");

  const loadBuiltins = () => {
    invoke<BuiltinGameInfo[]>("list_builtin_games")
      .then(setBuiltins)
      .catch(() => setBuiltins([]));
  };

  useEffect(() => {
    loadBuiltins();
  }, [games]);

  useEffect(() => {
    invoke<Record<string, string>>("get_custom_game_covers")
      .then(setCovers)
      .catch(() => setCovers({}));
  }, [bust]);

  // 合并：内置游戏（排除已隐藏）+ 用户自行添加的非内置游戏
  const builtinIds = new Set(builtins.map((b) => b.id));
  const visibleBuiltins = builtins.filter((b) => !b.hidden);
  const hiddenBuiltins = builtins.filter((b) => b.hidden);
  const cards: {
    id: string;
    name: string;
    modsPath: string;
    gbGameId: number;
    isBuiltin: boolean;
  }[] = [
    ...visibleBuiltins.map((b) => ({
      id: b.id,
      name: b.name,
      modsPath: b.mods_path,
      gbGameId: 0,
      isBuiltin: true,
    })),
    ...games
      .filter((g) => !builtinIds.has(g.id))
      .map((g) => ({
        id: g.id,
        name: g.name,
        modsPath: g.mods_path,
        gbGameId: g.gb_game_id ?? 0,
        isBuiltin: false,
      })),
  ];

  async function commitRename() {
    const id = editingId;
    const next = editValue.trim();
    setEditingId("");
    if (!id || !next) return;
    try {
      await invoke("rename_game", { id, name: next });
      await onRefreshGames();
      loadBuiltins();
    } catch (e) {
      alert(`重命名失败：${String(e)}`);
    }
  }

  /** 打开 GameBanana 市场绑定弹窗 */
  function openBinding(g: Game) {
    setBindingGame(g);
    setBindGameId(g.gb_game_id ? String(g.gb_game_id) : "");
    setBindCatId(g.gb_category_id ? String(g.gb_category_id) : "");
  }

  /** 保存市场绑定（游戏 ID 必填；分类根 ID 可留空，留空则按游戏 ID 列顶层分类） */
  async function commitBinding() {
    if (!bindingGame) return;
    const gid = Number(bindGameId.trim());
    const cid = bindCatId.trim() === "" ? 0 : Number(bindCatId.trim());
    if (!Number.isInteger(gid) || gid <= 0) {
      alert("GameBanana 游戏 ID 必须是正整数");
      return;
    }
    if (!Number.isInteger(cid) || cid < 0) {
      alert("分类根 ID 必须是非负整数");
      return;
    }
    try {
      await invoke("set_game_market_binding", {
        id: bindingGame.id,
        gbGameId: gid,
        gbCategoryId: cid,
      });
      setBindingGame(null);
      await onRefreshGames();
    } catch (e) {
      alert(`绑定失败：${String(e)}`);
    }
  }

  async function pickCover(id: string) {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] }],
      });
      if (typeof picked !== "string") return;
      await invoke("set_game_cover", { id, imagePath: picked });
      setBust((b) => b + 1);
    } catch (e) {
      alert(`更换封面失败：${String(e)}`);
    }
  }

  /** 统一移除：内置游戏走隐藏（可恢复），自定义游戏走真正移除 */
  async function removeCard(id: string) {
    if (builtinIds.has(id)) {
      await hideGame(id);
    } else {
      await onRemoveGame(id);
    }
  }

  /** 隐藏内置游戏（可从底部恢复） */
  async function hideGame(id: string) {
    try {
      await invoke("hide_builtin_game", { id });
      loadBuiltins();
      await onRefreshGames();
    } catch (e) {
      alert(`隐藏失败：${String(e)}`);
    }
  }

  /** 恢复显示隐藏的游戏 */
  async function unhideGame(id: string) {
    try {
      await invoke("unhide_builtin_game", { id });
      loadBuiltins();
      await onRefreshGames();
    } catch (e) {
      alert(`恢复失败：${String(e)}`);
    }
  }

  return (
    <div className="view">
      <div className="view-header">
        <div>
          <h2>切换游戏</h2>
          <div className="subtitle">
            已支持 {cards.length} 个游戏 · 在「系统设置 → 当前游戏」里配置各游戏的 Mods 路径
          </div>
        </div>
        <button className="primary" onClick={onAddGame}>＋ 添加其他游戏</button>
      </div>

      <div className="games-hint">
        <span className="games-hint-icon">💡</span>
        <div>
          <div className="games-hint-title">切换游戏后到系统设置配置 Mods 路径</div>
          <div className="games-hint-text">
            先切换到要玩的游戏，再到「系统设置 → 当前游戏」里选择该游戏目录下的 <strong>Mods</strong> 文件夹，
            例如 <code>XXMI MODS\EndField Game\Mods</code>
          </div>
        </div>
      </div>

      <div className="game-grid">
        {cards.map((g) => {
          const isActive = g.id === activeGameId;
          return (
            <div
              key={g.id}
              className={`game-card ${isActive ? "active" : ""}`}
            >
              <div className="game-cover">
                <GameCover
                  gameId={g.id}
                  fallback={g.name}
                  customPath={covers[g.id]}
                  bust={bust}
                />
                {isActive && <span className="game-active-tag">当前游戏</span>}
                <button
                  className="cover-edit-btn"
                  title="更换游戏封面"
                  onClick={(e) => {
                    e.stopPropagation();
                    pickCover(g.id);
                  }}
                >
                  📷
                </button>
              </div>
              <div className="game-body">
                {editingId === g.id ? (
                  <input
                    className="game-name-input"
                    autoFocus
                    value={editValue}
                    onChange={(e) => setEditValue(e.target.value)}
                    onBlur={() => setEditingId("")}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        commitRename();
                      } else if (e.key === "Escape") {
                        e.preventDefault();
                        setEditingId("");
                      }
                    }}
                    maxLength={40}
                  />
                ) : (
                  <div className="game-name">
                    <span className="game-name-text">{g.name}</span>
                    <button
                      className="name-edit-btn game-name-edit"
                      title="重命名游戏"
                      onClick={() => {
                        setEditingId(g.id);
                        setEditValue(g.name);
                      }}
                    >
                      ✏️
                    </button>
                  </div>
                )}

                <div className="game-actions">
                  {isActive ? (
                    <>
                      <button className="ghost" disabled>已是当前游戏</button>
                      <button className="danger" onClick={() => removeCard(g.id)}>
                        移除 / 隐藏
                      </button>
                    </>
                  ) : (
                    <>
                      <button className="primary" onClick={() => onSwitch(g.id)}>
                        切换到此游戏
                      </button>
                      <button className="danger" onClick={() => removeCard(g.id)}>
                        移除 / 隐藏
                      </button>
                    </>
                  )}
                  {!g.isBuiltin && (
                    <button
                      className="game-bind-btn"
                      title="绑定 GameBanana 游戏 ID 后即可使用 Mod 市场"
                      onClick={() => {
                        const full = games.find((x) => x.id === g.id);
                        if (full) openBinding(full);
                      }}
                    >
                      {g.gbGameId ? `🛒 市场已绑定 (${g.gbGameId})` : "🛒 绑定市场"}
                    </button>
                  )}
                </div>
              </div>
            </div>
          );
        })}

        {cards.length === 0 && (
          <div className="empty-hint">没有可见的游戏，可从下方恢复被隐藏的</div>
        )}
      </div>

      {/* 已隐藏的游戏（可恢复） */}
      {hiddenBuiltins.length > 0 && (
        <div className="hidden-games">
          <div className="hidden-games-title">
            已隐藏 ({hiddenBuiltins.length})
          </div>
          <div className="hidden-games-list">
            {hiddenBuiltins.map((c) => (
              <div key={c.id} className="hidden-game-item">
                <span className="hidden-game-name">{c.name}</span>
                <button onClick={() => unhideGame(c.id)}>恢复显示</button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* GameBanana 市场绑定弹窗 */}
      {bindingGame && (
        <div className="market-binding-mask" onClick={() => setBindingGame(null)}>
          <div className="market-binding" onClick={(e) => e.stopPropagation()}>
            <button className="market-binding-close" onClick={() => setBindingGame(null)} title="关闭">✕</button>
            <h3>🛒 绑定 GameBanana 市场</h3>
            <p className="market-binding-hint">
              为「{bindingGame.name}」绑定 GameBanana 游戏后，Mod 市场即可检索该游戏的 Mod。
              打开该游戏的 GameBanana 页面（如 <code>gamebanana.com/games/8552</code>），URL 里的数字就是游戏 ID。
            </p>
            <label className="market-binding-row">
              <span>游戏 ID <em>必填</em></span>
              <input
                type="number"
                min={1}
                value={bindGameId}
                placeholder="例如 8552"
                onChange={(e) => setBindGameId(e.target.value)}
              />
            </label>
            <label className="market-binding-row">
              <span>分类根 ID <em>可留空</em></span>
              <input
                type="number"
                min={0}
                value={bindCatId}
                placeholder="留空则按游戏 ID 列出分类"
                onChange={(e) => setBindCatId(e.target.value)}
              />
            </label>
            <p className="market-binding-hint">
              分类根 ID 用于展示角色分类列表（如原神的 18140）。留空时会自动列出 GameBanana 顶层分类，同样可以浏览和下载。
            </p>
            <div className="market-binding-actions">
              <button onClick={() => setBindingGame(null)}>取消</button>
              <button className="primary" onClick={commitBinding}>保存绑定</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ===== 游戏封面组件（使用前端静态资源） =====
const GAME_COVERS: Record<string, string> = {
  endfield: "/games/endfield.png",
  starrail: "/games/starrail.png",
  wuthering: "/games/wuthering.png",
  zenless: "/games/zenless.png",
};

function GameCover({
  gameId,
  fallback,
  customPath,
  bust,
}: {
  gameId: string;
  fallback: string;
  customPath?: string;
  bust?: number;
}) {
  const [failed, setFailed] = useState(false);
  const src = customPath
    ? convertFileSrc(customPath) + (bust ? `?v=${bust}` : "")
    : GAME_COVERS[gameId];
  if (!src || failed) {
    return <div className="game-cover-fallback">{fallback.slice(0, 4)}</div>;
  }
  return (
    <img
      src={src}
      alt={fallback}
      className="game-cover-img"
      onError={() => setFailed(true)}
    />
  );
}

// ===== 设置视图 =====
/** 检查更新接口的返回结构（check_app_update 命令） */
type AppUpdateInfo = {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseUrl: string;
  downloadUrl: string;
  publishedAt: string;
  notes: string;
};

/** 「关于」页面 */
function AboutView() {
  const AUTHOR_URL = "https://github.com/KobayashiWakaba1211";
  const APP_VERSION = "0.1.2";

  // 完整许可清单：悬浮预览 + 点击打开 txt
  const [licText, setLicText] = useState("");
  const [licHover, setLicHover] = useState(false);
  const [licPop, setLicPop] = useState<React.CSSProperties>({});
  const [licErr, setLicErr] = useState("");
  const licBtnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    invoke<string>("get_license_text")
      .then((t) => setLicText(t || ""))
      .catch(() => {});
  }, []);

  // 悬浮面板用 fixed 定位，避免被滚动容器裁切
  function showLicPop() {
    const el = licBtnRef.current;
    if (el) {
      const r = el.getBoundingClientRect();
      const width = Math.min(720, window.innerWidth - 32);
      const left = Math.max(16, Math.min(r.left, window.innerWidth - width - 16));
      const spaceBelow = window.innerHeight - r.bottom - 16;
      const openUp = spaceBelow < 320 && r.top > spaceBelow;
      setLicPop(
        openUp
          ? { left, bottom: window.innerHeight - r.top + 8, width }
          : { left, top: r.bottom + 8, width },
      );
    }
    setLicHover(true);
  }

  useEffect(() => {
    if (!licHover) return;
    const close = () => setLicHover(false);
    document.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [licHover]);

  function openLicenses() {
    setLicErr("");
    invoke<string>("open_license_file").catch((e) => setLicErr(String(e)));
  }

  // ===== 检查更新 =====
  const [updChecking, setUpdChecking] = useState(false);
  const [updInfo, setUpdInfo] = useState<AppUpdateInfo | null>(null);
  const [updErr, setUpdErr] = useState("");

  function checkUpdate() {
    if (updChecking) return;
    setUpdChecking(true);
    setUpdErr("");
    setUpdInfo(null);
    invoke<AppUpdateInfo>("check_app_update")
      .then(setUpdInfo)
      .catch((e) => setUpdErr(String(e)))
      .finally(() => setUpdChecking(false));
  }

  const features = [
    {
      icon: "👤",
      title: "角色一览",
      desc: "大量借鉴了 QAQM 的 UI 设计与功能，用于管理 Mod 的作用对象。卡片拖动排序需先在顶部选择「自定义」。",
    },
    {
      icon: "🛒",
      title: "Mod 市场",
      desc: "完全借鉴 IMM 的市场设计思路，调用 GameBanana 官方 API，支持一键下载安装、更新检测，以及「仅下载」。",
    },
    {
      icon: "💾",
      title: "预设配置",
      desc: "保存当前 Mod 的启用与禁用状态，方便在不同预设之间快速切换。",
    },
    {
      icon: "🎮",
      title: "切换游戏",
      desc: "默认支持五款游戏，也可自行添加。需在系统设置中配置该游戏的 Mod 文件夹与游戏程序路径。",
    },
    {
      icon: "🚀",
      title: "启动游戏",
      desc: "仿照 QAQM 制作的启动功能，需在系统设置中配置 XXMI Launcher 路径（全局，设置一次即可）。",
    },
  ];

  return (
    <div className="view">
      <div className="view-header">
        <div>
          <h2>关于</h2>
          <div className="subtitle">应用介绍、功能说明与作者信息</div>
        </div>
      </div>

      <div className="about-body">
        {/* 应用简介 */}
        <section className="about-section">
          <h3 className="settings-section-title">应用简介</h3>
          <div className="about-card">
            <div className="about-head">
              <img
                className="about-avatar"
                src="/about/avatar.jpg"
                alt="作者头像"
                draggable={false}
              />
              <div className="about-head-text">
                <div className="about-head-row">
                  <div className="about-logo">小林若葉Manager</div>
                  <span className="about-version">v{APP_VERSION}</span>
                </div>
              </div>
            </div>
            <p className="about-text">
              本应用主要用于管理 XXMI 模组。开发灵感来自 IMM 与 QAQM，在两者基础上取长补短、融合而成。
            </p>
            <p className="about-text">
              开发本意是降低使用门槛，让刚接触模组的新手也能轻松使用。
            </p>

            {/* 检查更新 */}
            <div className="update-row">
              <button className="panel-btn" onClick={checkUpdate} disabled={updChecking}>
                {updChecking ? "检查中…" : "🔄 检查更新"}
              </button>
              {updInfo && !updErr && (
                <span className={updInfo.hasUpdate ? "update-ok" : "update-latest"}>
                  {updInfo.hasUpdate
                    ? `发现新版本 v${updInfo.latestVersion}（当前 v${updInfo.currentVersion}）`
                    : `已是最新版本（v${updInfo.currentVersion}）`}
                </span>
              )}
              {updErr && <span className="update-err">检查失败：{updErr}</span>}
            </div>
            {updInfo?.hasUpdate && (
              <div className="update-panel">
                <div className="update-panel-head">
                  <span>
                    v{updInfo.latestVersion} · {updInfo.publishedAt || "发布日期未知"}
                  </span>
                  <button
                    className="panel-btn primary"
                    onClick={() => openUrl(updInfo.downloadUrl).catch(() => {})}
                    title={updInfo.downloadUrl}
                  >
                    ⬇ 下载安装包
                  </button>
                  <button
                    className="panel-btn"
                    onClick={() => openUrl(updInfo.releaseUrl).catch(() => {})}
                  >
                    打开发布页
                  </button>
                </div>
                {updInfo.notes && <pre className="update-notes">{updInfo.notes}</pre>}
              </div>
            )}
          </div>
        </section>

        {/* 主要功能 */}
        <section className="about-section">
          <h3 className="settings-section-title">主要功能</h3>
          <div className="about-grid">
            {features.map((f, i) => (
              <div className="about-feature" key={i}>
                <div className="about-feature-icon">{f.icon}</div>
                <div className="about-feature-text">
                  <div className="about-feature-title">{f.title}</div>
                  <div className="about-feature-desc">{f.desc}</div>
                </div>
              </div>
            ))}
          </div>
        </section>

        {/* 开发计划 */}
        <section className="about-section">
          <h3 className="settings-section-title">开发计划</h3>
          <div className="about-card">
            <ul className="about-list">
              <li>暂无进一步开发计划，目前仅计划逐步补全所支持游戏的缩略图。</li>
            </ul>
          </div>
        </section>

        {/* 使用说明 / 注意事项 */}
        <section className="about-section">
          <h3 className="settings-section-title">使用说明</h3>
          <div className="about-card">
            <ul className="about-list">
              <li>
                由于继承了 <strong>QAQM 的 Mod 结构</strong>，可以直接接管 QAQM 已有的配置，无需重新整理 Mod 目录。
              </li>
              <li>
                <strong>Persist Bridge 缓存</strong>功能同样可以无缝接管，切换过来后不需要重建。
              </li>
            </ul>
          </div>
        </section>

        {/* 开源组件与许可 */}
        <section className="about-section">
          <h3 className="settings-section-title">开源组件与许可</h3>
          <div className="about-card">
            <ul className="about-list">
              <li>
                解压能力全部内置（zip / 7z / rar），<strong>无需安装 7-Zip、Bandizip 或 WinRAR</strong>，拿到手即可直接用。
              </li>
              <li>
                RAR 解压使用 UnRAR 源码静态编译，依据其许可声明：
                <span className="about-license">
                  UnRAR source code may be used in any software to handle RAR archives without
                  limitations free of charge, but cannot be used to develop RAR (WinRAR) compatible
                  archiver and to re-create RAR compression algorithm, which is proprietary.
                </span>
                Copyright © Alexander Roshal。
              </li>
              <li>
                其余组件以 <strong>MIT / Apache-2.0</strong> 为主，另含 MPL-2.0、ISC、BSD-3-Clause、
                Unicode-3.0、Zlib 等宽松或弱著佐权许可，<strong>不含 GPL / AGPL</strong>。前端使用 React 19 与 Tauri 2。
              </li>
            </ul>
          </div>
          <div className="about-license-actions">
            <button
              ref={licBtnRef}
              className="about-license-link"
              onMouseEnter={showLicPop}
              onMouseLeave={() => setLicHover(false)}
              onFocus={showLicPop}
              onBlur={() => setLicHover(false)}
              onClick={openLicenses}
              title="点击用系统默认程序打开完整清单 txt"
            >
              📄 完整清单
            </button>
            <span className="about-license-hint">悬浮预览 · 点击打开 txt</span>
          </div>
          {licErr && <p className="about-footnote">打开失败：{licErr}</p>}
          {licHover && (
            <div className="about-license-pop" style={licPop} onMouseLeave={() => setLicHover(false)}>
              <div className="about-license-pop-head">
                完整清单预览（{licText ? `${licText.split("\n").length} 行` : "加载中…"}）· 点击上方按钮打开 txt
              </div>
              <pre className="about-license-pre">{licText || "加载中…"}</pre>
            </div>
          )}
        </section>

        {/* 作者与链接 */}
        <section className="about-section">
          <h3 className="settings-section-title">作者与链接</h3>
          <div className="about-card about-links">
            <button
              className="about-link-row"
              title="跳转作者主页"
              onClick={() => openUrl(AUTHOR_URL).catch(() => {})}
            >
              <span className="about-link-icon">🐙</span>
              <span className="about-link-label">作者主页</span>
              <span className="about-link-value">github.com/KobayashiWakaba1211 ↗</span>
            </button>
            <div className="about-link-row about-link-row-static">
              <span className="about-link-icon">📦</span>
              <span className="about-link-label">技术栈</span>
              <span className="about-link-value">Tauri 2 · React 19 · TypeScript</span>
            </div>
          </div>
          <p className="about-footnote">感谢使用。问题反馈与建议可通过作者主页联系。</p>
        </section>
      </div>
    </div>
  );
}

// ===== 个性化视图 =====
function PersonalizationView({
  cardSize,
  onCardSizeChange,
  galleryCardSize,
  onGalleryCardSizeChange,
  marketHeroScale,
  onMarketHeroScaleChange,
  marketCardSize,
  onMarketCardSizeChange,
}: {
  cardSize: number;
  onCardSizeChange: (size: number) => void;
  galleryCardSize: number;
  onGalleryCardSizeChange: (size: number) => void;
  marketHeroScale: number;
  onMarketHeroScaleChange: (size: number) => void;
  marketCardSize: number;
  onMarketCardSizeChange: (size: number) => void;
}) {
  // 主题偏好（深色/浅色/跟随系统）：立即应用到 <html data-theme> 并持久化
  const [themePref, setThemePref] = useState<ThemePref>(
    () => (localStorage.getItem("emm-theme-pref") as ThemePref) || "system",
  );
  // 跟随系统档：停留在本页时系统切换深浅色也要实时跟随
  useEffect(() => {
    if (themePref !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyThemePref("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [themePref]);
  function changeThemePref(next: ThemePref) {
    setThemePref(next);
    localStorage.setItem("emm-theme-pref", next);
    applyThemePref(next);
  }

  return (
    <div className="view">
      <div className="view-header">
        <div>
          <h2>个性化</h2>
          <div className="subtitle">主题配色与各处卡片显示大小</div>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">主题</h3>
        <div className="persist-card">
          <div className="cache-size-row">
            <label>配色主题：</label>
            <div className="theme-toggle">
              <button
                className={themePref === "dark" ? "active" : ""}
                onClick={() => changeThemePref("dark")}
              >
                🌙 深色
              </button>
              <button
                className={themePref === "light" ? "active" : ""}
                onClick={() => changeThemePref("light")}
              >
                ☀️ 浅色
              </button>
              <button
                className={themePref === "system" ? "active" : ""}
                onClick={() => changeThemePref("system")}
              >
                🖥️ 跟随系统
              </button>
            </div>
            <p className="cache-size-hint">
              界面配色主题。「跟随系统」会随 Windows 的深色/浅色模式实时切换。切换立即生效并自动保存。
            </p>
          </div>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">卡片大小</h3>
        <div className="persist-card">
          <div className="cache-size-row">
            <label>
              角色卡片大小：
              <strong>{cardSize}px</strong>
            </label>
            <input
              type="range"
              min={120}
              max={260}
              step={2}
              value={cardSize}
              onChange={(e) => onCardSizeChange(Number(e.target.value))}
            />
            <p className="cache-size-hint">
              角色一览卡片的显示宽度（{120}~{260}px）。行数和列数会随窗口大小和卡片大小自动增减，改动立即生效。
            </p>
          </div>
          <div className="cache-size-row">
            <label>
              画廊卡片大小：
              <strong>{galleryCardSize}px</strong>
            </label>
            <input
              type="range"
              min={140}
              max={320}
              step={2}
              value={galleryCardSize}
              onChange={(e) => onGalleryCardSizeChange(Number(e.target.value))}
            />
            <p className="cache-size-hint">
              角色详情页画廊视图的 Mod 卡片宽度（{140}~{320}px）。改动立即生效。
            </p>
          </div>
          <div className="cache-size-row">
            <label>
              市场大卡片大小：
              <strong>{marketHeroScale}%</strong>
            </label>
            <input
              type="range"
              min={40}
              max={100}
              step={5}
              value={marketHeroScale}
              onChange={(e) => onMarketHeroScaleChange(Number(e.target.value))}
            />
            <p className="cache-size-hint">
              Mod 市场首页顶部轮播推荐卡片的显示大小（{40}~{100}%，相对内容区宽度）。等比缩放：宽度变，高度按 11:4 比例跟随，卡片居中、两侧留白，封面不会变形。改动立即生效。
            </p>
          </div>
          <div className="cache-size-row">
            <label>
              市场小卡片大小：
              <strong>{marketCardSize}px</strong>
            </label>
            <input
              type="range"
              min={150}
              max={320}
              step={2}
              value={marketCardSize}
              onChange={(e) => onMarketCardSizeChange(Number(e.target.value))}
            />
            <p className="cache-size-hint">
              Mod 市场 Mod 网格卡片的显示宽度（{150}~{320}px），与「角色卡片大小」相同的自适应方式：列数随窗口宽度自动增减。改动立即生效。
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

function SettingsView({
  persistEnabled,
  persistCacheSize,
  onTogglePersist,
  onClearPersist,
  onCacheSizeChange,
  downloadDir,
  onChangeDownloadDir,
  activeGameName,
  activeModsPath,
  onChangeModsPath,
  xxmiPath,
  onChangeXxmiPath,
  launchConfig,
  onSaveLaunchConfig,
}: {
  persistEnabled: boolean;
  persistCacheSize: number;
  onTogglePersist: () => void;
  onClearPersist: () => void;
  onCacheSizeChange: (size: number) => void;
  downloadDir: string;
  onChangeDownloadDir: () => void;
  activeGameName: string;
  activeModsPath: string;
  onChangeModsPath: () => void;
  xxmiPath: string;
  onChangeXxmiPath: () => void;
  launchConfig: LaunchConfig | null;
  onSaveLaunchConfig: (patch: {
    game_path?: string;
    launcher_path?: string;
    giml_path?: string;
    launch_mode?: string;
    launch_args?: string;
  }) => void;
}) {
  // 手动输入框的本地值（失焦/回车时才提交，避免每敲一下都重建）
  const [cacheInput, setCacheInput] = useState(String(persistCacheSize));

  // 启动设置编辑值（复刻 QAQM：附加启动参数）
  const [launchArgs, setLaunchArgs] = useState("");
  useEffect(() => {
    setLaunchArgs(launchConfig?.launch_args ?? "");
  }, [launchConfig]);

  async function pickLaunchExe(kind: "game" | "launcher" | "giml") {
    const picked = await open({
      multiple: false,
      filters: [{ name: "程序", extensions: ["exe", "cmd", "bat"] }],
    });
    if (typeof picked !== "string") return;
    if (kind === "game") {
      onSaveLaunchConfig({ game_path: picked });
    } else if (kind === "giml") {
      onSaveLaunchConfig({ giml_path: picked });
    } else {
      onSaveLaunchConfig({ launcher_path: picked });
    }
  }

  useEffect(() => {
    setCacheInput(String(persistCacheSize));
  }, [persistCacheSize]);

  const commitCacheInput = () => {
    const n = Number(cacheInput);
    if (cacheInput.trim() === "" || !Number.isFinite(n)) {
      setCacheInput(String(persistCacheSize));
      return;
    }
    const clamped = Math.max(0, Math.min(50, Math.round(n)));
    setCacheInput(String(clamped));
    if (clamped !== persistCacheSize) {
      onCacheSizeChange(clamped);
    }
  };

  return (
    <div className="view">
      <div className="view-header">
        <div>
          <h2>系统设置</h2>
          <div className="subtitle">当前游戏、下载与 Persist Bridge 配置</div>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">当前游戏：{activeGameName}</h3>
        <div className="persist-card">
          <div className="persist-row download-dir-row">
            <span className="download-dir-label">
              Mods 文件夹：
              <strong className="download-dir-path" title={activeModsPath}>
                {activeModsPath || "（尚未设置）"}
              </strong>
            </span>
            <button className="panel-btn" onClick={onChangeModsPath}>
              📁 更改文件夹
            </button>
          </div>
          <div className="persist-row download-dir-row">
            <span className="download-dir-label">
              游戏程序路径：
              <strong className="download-dir-path" title={launchConfig?.game_path}>
                {launchConfig?.game_path || "（未设置，原版启动不可用）"}
              </strong>
            </span>
            <button className="panel-btn" onClick={() => pickLaunchExe("game")}>
              📁 选择
            </button>
          </div>
          <p className="persist-desc">
            以上两项<strong>跟随当前游戏切换</strong>，换游戏后会自动显示对应游戏的配置。
            Mods 文件夹请选择该游戏目录下的 <strong>Mods</strong> 文件夹，例如{" "}
            <code>XXMI MODS\EndField Game\Mods</code>；游戏程序路径留空时会自动从 XXMI Launcher 配置推断。
          </p>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">启动设置：{activeGameName}</h3>
        <div className="persist-card">
          {(launchConfig?.supported_modes ?? []).includes("GIML") && (
            <div className="persist-row download-dir-row">
              <span className="download-dir-label">
                GIML.exe 路径：
                <strong className="download-dir-path" title={launchConfig?.giml_path}>
                  {launchConfig?.giml_path || "（未设置，自动在桌面 / 下载目录里找）"}
                </strong>
              </span>
              <button className="panel-btn" onClick={() => pickLaunchExe("giml")}>
                📁 选择
              </button>
            </div>
          )}
          <div className="persist-row download-dir-row">
            <span className="download-dir-label">
              附加启动参数：
              <input
                className="launch-args-input"
                value={launchArgs}
                placeholder="例如 -dx11"
                onChange={(e) => setLaunchArgs(e.target.value)}
                onBlur={() => onSaveLaunchConfig({ launch_args: launchArgs })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onSaveLaunchConfig({ launch_args: launchArgs });
                }}
              />
            </span>
          </div>
          <p className="persist-desc">
            游戏 exe 路径留空时会自动从 XXMI Launcher 配置推断。
            <strong>EFMI / XXMI</strong> 等注入模式通过下面配置的 XXMI Launcher 带{" "}
            <code>--nogui --xxmi</code> 参数启动；<strong>原版启动</strong>直接运行游戏程序。
            <strong>GIML</strong>（原神）直接运行 GIML.exe 加壳启动器，路径留空时自动在桌面 / 下载目录的 GIML
            文件夹里查找。模式在左侧栏底部切换。
          </p>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">XXMI Launcher（全局）</h3>
        <div className="persist-card">
          <div className="persist-row download-dir-row">
            <span className="download-dir-label">
              XXMI 启动器路径：
              <strong className="download-dir-path" title={xxmiPath}>
                {xxmiPath || "（未设置，注入模式不可用）"}
              </strong>
            </span>
            <button className="panel-btn" onClick={onChangeXxmiPath}>
              📁 选择
            </button>
          </div>
          <p className="persist-desc">
            这是<strong>全局设置，配置一次所有游戏共用</strong>，切换游戏时不会变化。
            留空时会自动从 XXMI Launcher 的默认安装位置推断。
          </p>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">下载</h3>
        <div className="persist-card">
          <div className="persist-row download-dir-row">
            <span className="download-dir-label">
              仅下载保存文件夹：
              <strong className="download-dir-path" title={downloadDir}>
                {downloadDir || "（系统默认下载文件夹）"}
              </strong>
            </span>
            <button className="panel-btn" onClick={onChangeDownloadDir}>
              📁 更改文件夹
            </button>
          </div>
          <p className="persist-desc">
            市场详情里「仅下载」的保存位置。默认为当前系统的下载文件夹，更改后立即生效。
          </p>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">Persist Bridge</h3>
        <div className="persist-card">
          <div className="persist-row">
            <span>启用 Persist Bridge</span>
            <label className="switch">
              <input type="checkbox" checked={persistEnabled} onChange={onTogglePersist} />
              <span className="slider" />
            </label>
          </div>
        <p className="persist-desc">
          开启后，管理器接管 mod 的 global persist 变量，使 mod 切走再切回后，部件切换状态（如裙子、上衣、饰品）不丢失。
          <br />开启或关闭后需重启游戏才生效。
        </p>

        <div className="cache-size-row">
          <label>
            缓存保留数量：
            <input
              type="number"
              min={0}
              max={50}
              value={cacheInput}
              onChange={(e) => setCacheInput(e.target.value)}
              onBlur={commitCacheInput}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.currentTarget.blur();
                }
              }}
            />
            个 Mod
          </label>
          <input
            type="range"
            min={0}
            max={50}
            step={1}
            value={persistCacheSize}
            onChange={(e) => onCacheSizeChange(Number(e.target.value))}
          />
          <p className="cache-size-hint">
            除当前启用的 Mod 外，额外记住最近切换过的 Mod 部件状态。0 = 只保留当前启用的。
            数值越大占用越多（每个 Mod 平均 50 个 persist 变量），上限 50。改动立即生效。
          </p>
        </div>

        <button className="clear-cache" onClick={onClearPersist}>🧹 清空 Persist 缓存</button>
        </div>
      </div>
    </div>
  );
}

// ===== 封面图组件（使用 asset 协议，避免 base64 撑爆 DOM） =====
function Cover({
  basePath,
  character,
  cover,
  bust,
  gameId,
}: {
  basePath: string;
  character: string;
  cover: string;
  /** 变更戳：改变时强制重新加载图片（绕过缓存） */
  bust?: number;
  /** 游戏 id：用于回退到打包进程序的内置封面 */
  gameId?: string;
}) {
  // 本地封面（用户自定义/目录自带）加载失败后，依次回退：内置封面 → emoji
  const [stage, setStage] = useState<"local" | "builtin" | "emoji">("local");
  const fullPath = `${basePath}/${character}/${cover}`.replace(/\\/g, "/");
  const localSrc = convertFileSrc(fullPath) + (bust ? `?v=${bust}` : "");
  const hasBuiltin = !!gameId && (BUILTIN_COVERS[gameId] || []).includes(character);
  const builtinSrc = hasBuiltin
    ? `/character-covers/${encodeURIComponent(gameId!)}/${encodeURIComponent(character)}.webp`
    : "";

  if (stage === "emoji" || (!cover && !builtinSrc)) {
    return <div className="char-emoji">🎭</div>;
  }
  const src = stage === "local" && cover ? localSrc : builtinSrc;
  return (
    <img
      src={src}
      alt={character}
      className="cover-img"
      loading="lazy"
      draggable={false}
      onError={() => {
        if (stage === "local" && builtinSrc) setStage("builtin");
        else setStage("emoji");
      }}
    />
  );
}

/** ArrayBuffer → base64（分块，避免大图爆栈） */
function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** 从内置皮肤目录里查某个角色的官方皮肤 */
function findCharacterSkins(
  catalog: Record<string, { characters: any[] }>,
  gameId: string,
  charName: string
): OfficialSkin[] {
  const gameData = catalog?.[gameId];
  if (!gameData?.characters) return [];
  const norm = (s: string) =>
    String(s || "").trim().toLowerCase().replace(/\s+/g, "");
  const key = norm(charName);
  const found = gameData.characters.find((c: any) => {
    if (norm(c.nameZh) === key) return true;
    if (c.nameEn && norm(c.nameEn) === key) return true;
    return (c.aliases || []).some((a: string) => norm(a) === key);
  });
  return (found?.skins || []) as OfficialSkin[];
}

export default App;
