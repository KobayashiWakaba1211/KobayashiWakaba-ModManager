import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { BUILTIN_COVERS } from "./builtinCovers";
import "./Overlay.css";

type Mod = { name: string; character: string; enabled: boolean; rel_path: string; cover: string; folder_name: string; modified: number };
type Group = { name: string; cover: string; mods: Mod[]; enabled_count: number; total_count: number };

/** 两次刷新之间的最小间隔（ms）：合并事件风暴，避免高频全量扫描 */
const REFRESH_MIN_INTERVAL = 800;
const SCALE_MIN = 0.85;
const SCALE_MAX = 1.45;
const SCALE_STEP = 0.1;

type SortKey = "name" | "enabled" | "time" | "custom";
type ViewMode = "gallery" | "list";
type CharFilter = "all" | "hasMods" | "noMods";

// ===== 主题（与管理器共用偏好：emm-theme-pref） =====

function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/** 按偏好设置本窗口 <html data-theme>；system 档实时解析系统深浅色 */
function applyOverlayTheme() {
  const saved = localStorage.getItem("emm-theme-pref");
  const pref = saved === "dark" || saved === "light" || saved === "system" ? saved : "system";
  document.documentElement.dataset.theme =
    pref === "system" ? (systemPrefersDark() ? "dark" : "light") : pref;
}

// 模块加载即应用，避免深浅色闪变
applyOverlayTheme();

export default function Overlay() {
  const [gameId, setGameId] = useState("");
  const [path, setPath] = useState("");
  const [groups, setGroups] = useState<Group[]>([]);
  const [character, setCharacter] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [debug, setDebug] = useState("待刷新");
  const [singleMode, setSingleMode] = useState(false);
  const [pinned, setPinned] = useState<string[]>([]);
  const [hotkey, setHotkey] = useState("Alt+F");

  // UI 状态（仿 QAQM）
  const [scale, setScale] = useState(() => clampScale(Number(localStorage.getItem("qaqm.overlayScale")) || 1));
  const [winPinned, setWinPinned] = useState(true);
  const [charPanelOpen, setCharPanelOpen] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey>("name");
  const [viewMode, setViewMode] = useState<ViewMode>(() =>
    localStorage.getItem("qaqm.overlayView") === "list" ? "list" : "gallery"
  );
  const [charFilter, setCharFilter] = useState<CharFilter>("all");

  const running = useRef(false);
  const dirty = useRef(false);
  const token = useRef(0);
  const mounted = useRef(false);
  const lastRunAt = useRef(0);
  const timer = useRef<number | null>(null);
  const chipsRef = useRef<HTMLDivElement | null>(null);
  const cardsRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const avatarGridRef = useRef<HTMLDivElement | null>(null);

  // 角色标签行：滚轮纵向滚动 → 横向滚动（无滚动条）
  useEffect(() => {
    const el = chipsRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY === 0 || el.scrollWidth <= el.clientWidth) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Mod 卡片区 / 列表区 / 头像面板：滚轮上下滚动（显式接管，缩放后也保持可用）
  useEffect(() => {
    const els = [cardsRef.current, listRef.current, avatarGridRef.current].filter(
      (el): el is HTMLDivElement => el !== null
    );
    const cleanups = els.map((el) => {
      const onWheel = (e: WheelEvent) => {
        if (el.scrollHeight <= el.clientHeight) return;
        e.preventDefault();
        el.scrollTop += e.deltaY;
      };
      el.addEventListener("wheel", onWheel, { passive: false });
      return () => el.removeEventListener("wheel", onWheel);
    });
    return () => cleanups.forEach((fn) => fn());
  }, [viewMode, charPanelOpen]);

  function clampScale(v: number) {
    if (!Number.isFinite(v)) return 1;
    return Math.min(SCALE_MAX, Math.max(SCALE_MIN, Math.round(v * 10) / 10));
  }

  async function load(version: number) {
    // 游戏切换在主界面完成，悬浮窗只读取当前激活的游戏
    const active = await invoke<string>("get_active_game_id");
    const [modsPath, single, pinnedList] = await Promise.all([
      invoke<string>("get_mods_path"),
      invoke<boolean>("get_single_mode"),
      invoke<string[]>("get_pinned_mods"),
    ]);
    if (!mounted.current || token.current !== version || document.visibilityState !== "visible") {
      dirty.current = true;
      return false;
    }
    const nextGroups = modsPath ? await invoke<Group[]>("scan") : [];
    const currentActive = await invoke<string>("get_active_game_id");
    const currentPath = await invoke<string>("get_mods_path");
    if (!mounted.current || token.current !== version) return false;
    if (document.visibilityState !== "visible" || active !== currentActive || modsPath !== currentPath) {
      dirty.current = true;
      return false;
    }
    setGameId(active);
    setPath(modsPath);
    setGroups(nextGroups);
    setSingleMode(single);
    setPinned(pinnedList || []);
    setCharacter((current) => nextGroups.some((group) => group.name === current) ? current : nextGroups[0]?.name || "");
    return true;
  }

  async function doRun(source: string) {
    if (document.visibilityState !== "visible") {
      dirty.current = true;
      return;
    }
    lastRunAt.current = Date.now();
    if (running.current) {
      dirty.current = true;
      return;
    }
    running.current = true;
    const version = ++token.current;
    setBusy(true);
    try {
      if (await load(version) && mounted.current && token.current === version) {
        setError("");
        setDebug(`${source} · #${version}`);
      }
    } catch (e) {
      if (mounted.current && token.current === version) setError(String(e));
    } finally {
      running.current = false;
      if (!mounted.current) return;
      setBusy(false);
      if (dirty.current && document.visibilityState === "visible") {
        dirty.current = false;
        schedule("合并刷新");
      }
    }
  }

  /** 事件触发的刷新：合并风暴 + 节流；悬浮窗隐藏时不做全量扫描 */
  function schedule(source: string) {
    if (!mounted.current) return;
    if (running.current) {
      dirty.current = true;
      token.current++;
      return;
    }
    if (document.visibilityState !== "visible") {
      dirty.current = true;
      return;
    }
    const gap = Date.now() - lastRunAt.current;
    if (gap < REFRESH_MIN_INTERVAL) {
      if (timer.current == null) {
        timer.current = window.setTimeout(() => {
          timer.current = null;
          if (mounted.current && document.visibilityState === "visible") void doRun(source);
          else if (mounted.current) dirty.current = true;
        }, REFRESH_MIN_INTERVAL - gap);
      }
      return;
    }
    void doRun(source);
  }

  /** 用户操作（切换游戏 / 启停 Mod / 置顶 / 单选）：执行动作 → 通知主窗口 → 重新读取 */
  async function runAction(fn: () => Promise<unknown>, label: string) {
    if (running.current) return;
    running.current = true;
    const version = ++token.current;
    setBusy(true);
    setError("");
    try {
      await fn();
      await invoke("notify_overlay_state_changed");
      if (await load(version) && mounted.current && token.current === version) {
        setError("");
        setDebug(`${label} · #${version}`);
      }
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      running.current = false;
      if (!mounted.current) return;
      setBusy(false);
      if (dirty.current && document.visibilityState === "visible") {
        dirty.current = false;
        schedule("合并刷新");
      }
    }
  }

  useEffect(() => {
    mounted.current = true;
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    // 作用域限定监听：只接收显式发给本窗口的事件，杜绝收到别的窗口的通知造成互相触发
    const webview = getCurrentWebviewWindow();
    const refreshHotkey = () => {
      void invoke<string>("get_overlay_hotkey")
        .then((value) => { if (!cancelled) setHotkey(value); })
        .catch((e) => { if (!cancelled) console.warn("读取悬浮窗快捷键失败", e); });
    };
    void webview.listen("overlay-hotkey-changed", refreshHotkey).then((unlisten) => {
      if (!cancelled) unlisteners.push(unlisten);
      else unlisten();
    }).catch((e) => { if (!cancelled) console.warn("监听悬浮窗快捷键失败", e); });
    refreshHotkey();
    for (const event of ["overlay-refresh", "overlay-state-changed"] as const) {
      void webview.listen(event, () => schedule(event)).then((unlisten) => {
        if (!cancelled) unlisteners.push(unlisten);
        else unlisten();
      }).catch((e) => { if (!cancelled) setError(String(e)); });
    }
    void doRun("启动");
    const onFocus = () => { refreshHotkey(); schedule("获得焦点"); };
    const onVisibility = () => {
      if (document.visibilityState === "visible" && dirty.current) {
        dirty.current = false;
        schedule("重新可见");
      }
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    // 主题跟随：系统深浅色变化（system 档）+ 主窗口改了主题偏好（storage 事件跨窗口通知）
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onSchemeChange = () => applyOverlayTheme();
    mq.addEventListener("change", onSchemeChange);
    const onStorage = (e: StorageEvent) => {
      if (e.key === "emm-theme-pref" || e.key === "emm-theme") applyOverlayTheme();
    };
    window.addEventListener("storage", onStorage);
    return () => {
      cancelled = true;
      mounted.current = false;
      token.current++;
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      mq.removeEventListener("change", onSchemeChange);
      window.removeEventListener("storage", onStorage);
      if (timer.current != null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
      unlisteners.forEach((unlisten) => unlisten());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ===== 派生数据 =====
  const visibleGroups = groups.filter((g) => {
    if (charFilter === "hasMods") return g.total_count > 0;
    if (charFilter === "noMods") return g.total_count === 0;
    return true;
  });
  const group = groups.find((item) => item.name === character);
  const sortedMods: Mod[] = (() => {
    const mods = group ? [...group.mods] : [];
    switch (sortKey) {
      case "name": mods.sort((a, b) => a.name.localeCompare(b.name, "zh")); break;
      case "enabled": mods.sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.name.localeCompare(b.name, "zh")); break;
      case "time": mods.sort((a, b) => b.modified - a.modified); break;
      case "custom": break; // 扫描顺序
    }
    return mods;
  })();
  const totals = groups.reduce(
    (acc, g) => ({ enabled: acc.enabled + g.enabled_count, total: acc.total + g.total_count }),
    { enabled: 0, total: 0 }
  );

  function modCoverSrc(mod: Mod) {
    if (mod.cover && path) {
      return convertFileSrc(`${path}/${mod.character}/${mod.folder_name}/${mod.cover}`.replace(/\\/g, "/"));
    }
    return "";
  }

  /** 角色头像：本地封面 → 内置封面 → 首字 */
  function charAvatarSrc(name: string, cover: string) {
    if (cover && path) {
      return convertFileSrc(`${path}/${name}/${cover}`.replace(/\\/g, "/"));
    }
    if ((BUILTIN_COVERS[gameId] || []).includes(name)) {
      return `/character-covers/${encodeURIComponent(gameId)}/${encodeURIComponent(name)}.webp`;
    }
    return "";
  }

  function changeScale(delta: number) {
    setScale((prev) => {
      const next = clampScale(prev + delta);
      localStorage.setItem("qaqm.overlayScale", String(next));
      return next;
    });
  }

  function switchViewMode(mode: ViewMode) {
    setViewMode(mode);
    localStorage.setItem("qaqm.overlayView", mode);
  }

  function toggleWindowPin() {
    const next = !winPinned;
    setWinPinned(next);
    getCurrentWebviewWindow().setAlwaysOnTop(next).catch((e) => {
      setWinPinned(!next);
      setError(String(e));
    });
  }

  return (
    <div className="overlay" style={{ zoom: scale, "--oz": scale } as React.CSSProperties}>
      <header className="overlay-topbar">
        <span className="overlay-logo" aria-hidden>🎮</span>
        <strong className="overlay-title">Mod 切换</strong>
        <div className="overlay-top-actions">
          <button className="overlay-tbtn" onClick={() => changeScale(-SCALE_STEP)} disabled={scale <= SCALE_MIN} title="缩小界面">A-</button>
          <span className="overlay-scale">{Math.round(scale * 100)}%</span>
          <button className="overlay-tbtn" onClick={() => changeScale(SCALE_STEP)} disabled={scale >= SCALE_MAX} title="放大界面">A+</button>
          <button className={`overlay-tbtn ${winPinned ? "active" : ""}`} onClick={toggleWindowPin} title="窗口置顶开关">📌</button>
          <button className="overlay-tbtn close" onClick={() => invoke("hide_overlay").catch((e) => setError(String(e)))} title="关闭（Alt+F 可再次呼出）">×</button>
        </div>
      </header>

      {charPanelOpen && (
        <>
          <div className="overlay-backdrop" onClick={() => setCharPanelOpen(false)} />
          <aside className="overlay-sidepanel">
            <div className="overlay-sidepanel-title">选择角色</div>
            <div className="overlay-avatar-grid" ref={avatarGridRef}>
              {groups.map((item) => {
                const src = charAvatarSrc(item.name, item.cover);
                return (
                  <button key={item.name}
                    className={`overlay-avatar-item ${item.name === group?.name ? "selected" : ""}`}
                    title={item.name}
                    onClick={() => {
                      if (!visibleGroups.some((visible) => visible.name === item.name)) setCharFilter("all");
                      setCharacter(item.name);
                      setCharPanelOpen(false);
                    }}>
                    <span className="overlay-avatar-circle">
                      {src ? <img src={src} alt="" draggable={false} onError={(e) => { e.currentTarget.style.display = "none"; }} /> : item.name.slice(0, 1)}
                    </span>
                    <span className="overlay-avatar-name">{item.name}</span>
                  </button>
                );
              })}
            </div>
          </aside>
        </>
      )}

      <div className="overlay-char-row">
        <button className={`overlay-menu-btn ${charPanelOpen ? "active" : ""}`} onClick={() => setCharPanelOpen((v) => !v)} title="选择角色">≡</button>
        <div className="overlay-chips" ref={chipsRef}>
          {visibleGroups.map((item) => (
            <button key={item.name}
              className={`overlay-chip ${item.name === group?.name ? "active" : ""}`}
              onClick={() => setCharacter(item.name)} disabled={busy} title={item.name}>
              <span className="chip-name">{item.name}</span>
              <span className="chip-cnt">{item.enabled_count}/{item.total_count}</span>
            </button>
          ))}
          {!visibleGroups.length && <span className="overlay-inline-empty">{busy ? "正在加载…" : "暂无角色"}</span>}
        </div>
      </div>

      <div className="overlay-toolbar">
        <span className="overlay-lbl">排序</span>
        {([["name", "名称"], ["enabled", "状态"], ["time", "时间"], ["custom", "自定义"]] as [SortKey, string][]).map(([k, label]) => (
          <button key={k} className={`overlay-chip small ${sortKey === k ? "active" : ""}`} onClick={() => setSortKey(k)}>{label}</button>
        ))}
        <span className="overlay-lbl gap">🔍 定位</span>
        <button className={`overlay-chip small ${viewMode === "gallery" ? "active" : ""}`} onClick={() => switchViewMode("gallery")}>合集</button>
        <button className={`overlay-chip small ${viewMode === "list" ? "active" : ""}`} onClick={() => switchViewMode("list")}>列表</button>
        <button className={`overlay-chip small ${singleMode ? "active" : ""}`} title="启用一个 Mod 时自动停用同角色其他 Mod"
          onClick={() => void runAction(() => invoke("set_single_mode", { enabled: !singleMode }), "单选")}>
          {singleMode ? "◉" : "○"} 单选
        </button>
      </div>

      {error && <div className="overlay-error" role="alert">{error}</div>}

      {viewMode === "gallery" ? (
        <div className="overlay-cards" ref={cardsRef}>
          {sortedMods.map((mod) => {
            const isPinned = pinned.includes(mod.rel_path);
            const src = modCoverSrc(mod);
            return (
              <div key={mod.rel_path} className={`overlay-card ${mod.enabled ? "on" : ""}`}>
                <button className={`overlay-card-pin ${isPinned ? "pinned" : ""}`} title={isPinned ? "取消置顶" : "置顶该 Mod"}
                  disabled={busy}
                  onClick={() => void runAction(async () => {
                    if (await invoke<string>("get_active_game_id") !== gameId) throw new Error("当前游戏已变化，请刷新后重试");
                    return invoke("toggle_mod_pin", { character: mod.character, modName: mod.name });
                  }, "置顶")}>
                  📌
                </button>
                <div className="overlay-card-img">
                  {src ? <img src={src} alt="" draggable={false} onError={(e) => { e.currentTarget.style.display = "none"; }} /> : <span className="pkg">📦</span>}
                </div>
                <div className="overlay-card-bottom">
                  <span className="overlay-card-name" title={mod.name}>{mod.name}</span>
                  <button className={`overlay-switch ${mod.enabled ? "on" : ""}`} role="switch" aria-checked={mod.enabled}
                    aria-label={`${mod.enabled ? "停用" : "启用"} ${mod.name}`} disabled={busy}
                    onClick={() => void runAction(async () => {
                      if (await invoke<string>("get_active_game_id") !== gameId) throw new Error("当前游戏已变化，请刷新后重试");
                      return singleMode && !mod.enabled
                        ? invoke("enable_mod_exclusive", { character: mod.character, modName: mod.name })
                        : invoke("toggle_mod", { character: mod.character, modName: mod.name, enabled: !mod.enabled });
                    }, "切换 Mod")}>
                    <span />
                  </button>
                </div>
              </div>
            );
          })}
          {group && !sortedMods.length && <div className="overlay-empty">该角色暂无 Mod</div>}
          {!group && <div className="overlay-empty">{busy ? "正在加载…" : path ? "暂无角色" : "请先在主界面配置 Mods 目录"}</div>}
        </div>
      ) : (
        <div className="overlay-list" ref={listRef}>
          {sortedMods.map((mod) => (
            <div key={mod.rel_path} className={`overlay-row ${mod.enabled ? "on" : ""}`}>
              {pinned.includes(mod.rel_path) && <span className="row-pin" title="已置顶">📌</span>}
              <span className="overlay-row-name" title={mod.name}>{mod.name}</span>
              <button className={`overlay-switch ${mod.enabled ? "on" : ""}`} role="switch" aria-checked={mod.enabled}
                aria-label={`${mod.enabled ? "停用" : "启用"} ${mod.name}`} disabled={busy}
                onClick={() => void runAction(async () => {
                  if (await invoke<string>("get_active_game_id") !== gameId) throw new Error("当前游戏已变化，请刷新后重试");
                  return singleMode && !mod.enabled
                    ? invoke("enable_mod_exclusive", { character: mod.character, modName: mod.name })
                    : invoke("toggle_mod", { character: mod.character, modName: mod.name, enabled: !mod.enabled });
                }, "切换 Mod")}>
                <span />
              </button>
            </div>
          ))}
          {group && !sortedMods.length && <div className="overlay-empty">该角色暂无 Mod</div>}
          {!group && <div className="overlay-empty">{busy ? "正在加载…" : path ? "暂无角色" : "请先在主界面配置 Mods 目录"}</div>}
        </div>
      )}

      <div className="overlay-filter">
        <span className="overlay-lbl">👑 筛选</span>
        {([["all", "全部"], ["hasMods", "有 Mod"], ["noMods", "无 Mod"]] as [CharFilter, string][]).map(([k, label]) => (
          <button key={k} className={`overlay-chip small ${charFilter === k ? "active" : ""}`} onClick={() => setCharFilter(k)}>{label}</button>
        ))}
      </div>

      <footer className="overlay-footer">
        <span title="诊断：最近一次刷新来源">{hotkey} 显示 / 隐藏 · ≡ 选角色 · 切游戏请回主界面 · {debug}</span>
        <span className="overlay-total">{totals.enabled}/{totals.total}</span>
      </footer>
    </div>
  );
}
