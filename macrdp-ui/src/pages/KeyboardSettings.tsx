import { useEffect, useId, useRef, useState } from "react";
import { ArrowRight, Keyboard, Loader2, Pencil, Plus, Search, Trash2 } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/ipc";
import type { KeyChord, KeyMapping, KeyModifier, KeyOption, RecordedKey } from "../lib/types";
import { ChordRecorder, keyIdFromCode, modifiers } from "../lib/keyboard-recording";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";

type Side = "from" | "to";
const sideLabels = { from: "原按键", to: "映射为" };
const modifierLabels: Record<Side, Record<KeyModifier, string>> = {
  from: { control: "Ctrl", alt: "Alt", shift: "Shift", meta: "Meta" },
  to: { control: "Control", alt: "Option", shift: "Shift", meta: "Command" },
};
let recordingSequence = 0;

function keyLabel(key: KeyOption, side: Side) {
  return side === "from" ? key.windows.replace("Win", "Meta") : key.mac;
}

function chordLabel(chord: KeyChord, side: Side, keys: KeyOption[]) {
  const key = keys.find((key) => key.id === chord.key);
  return [...modifiers.filter((modifier) => chord.modifiers.includes(modifier))
    .map((modifier) => modifierLabels[side][modifier]), key ? keyLabel(key, side) : chord.key].join(" + ");
}

function ChordEditor({ side, chord, keys, disabled, onChange }: {
  side: Side;
  chord: KeyChord;
  keys: KeyOption[];
  disabled: boolean;
  onChange: (chord: KeyChord) => void;
}) {
  const id = useId();
  const [recording, setRecording] = useState<number | null>(null);
  const [ready, setReady] = useState(false);
  const [preview, setPreview] = useState<KeyChord | null>(null);
  const [hint, setHint] = useState("");
  const [picker, setPicker] = useState<KeyChord | null>(null);
  const [search, setSearch] = useState("");
  const pickerButtonRef = useRef<HTMLButtonElement>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (recording === null) return;
    let disposed = false;
    let enabled = false;
    const recorder = new ChordRecorder();
    const unlisteners: (() => void)[] = [];
    const stop = () => { enabled = false; setRecording(null); };
    const accept = (key: string | null, pressed: boolean, activeModifiers: KeyModifier[] = []) => {
      if (!enabled || disposed) return;
      if (key === "escape" || key === "tab") { stop(); return; }
      if (!key || !keys.some((option) => option.id === key)) {
        setHint("无法识别这个按键，请使用“选择按键”。");
        return;
      }
      if (pressed) {
        setPreview(recorder.keyDown(key, activeModifiers));
      } else {
        const result = recorder.keyUp(key);
        if (result) { onChangeRef.current(result); stop(); }
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.code === "Tab") { stop(); return; }
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.repeat) return;
      const activeModifiers = modifiers.filter((modifier) => ({
        control: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey,
      })[modifier]);
      accept(keyIdFromCode(event.code) ?? null, event.type === "keydown", activeModifiers);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("blur", stop);
    window.addEventListener("pointerdown", stop, true);
    const onVisibility = () => { if (document.hidden) stop(); };
    document.addEventListener("visibilitychange", onVisibility);
    const register = async () => {
      if (side === "from") {
        const unlisten = await listen<RecordedKey>("recorded-key", ({ payload }) => {
          if (payload.session !== recording) return;
          if (payload.key === "tab" && payload.pressed) pickerButtonRef.current?.focus();
          accept(payload.key, payload.pressed);
        });
        if (disposed) { unlisten(); return; }
        unlisteners.push(unlisten);
        await api.setKeyboardRecording(recording);
        if (disposed) { await api.setKeyboardRecording(null); return; }
      }
      const unlisten = await listen("connection-event", stop);
      if (disposed) { unlisten(); return; }
      unlisteners.push(unlisten);
      enabled = true;
      setReady(true);
    };
    void register().catch((error: unknown) => {
      if (!disposed) { setHint(`无法开始录入：${String(error)}`); stop(); }
    });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("blur", stop);
      window.removeEventListener("pointerdown", stop, true);
      document.removeEventListener("visibilitychange", onVisibility);
      if (side === "from") void api.setKeyboardRecording(null).catch((error: unknown) => setHint(`停止录入失败：${String(error)}`));
    };
  }, [recording, side, keys]);

  const filtered = keys.filter((key) => `${key.windows} ${key.mac} ${key.id} ${key.group}`.toLowerCase().includes(search.toLowerCase().trim()));
  const groups = [...new Set(filtered.map((key) => key.group))];
  return (
    <div className="min-w-0 space-y-2">
      <label htmlFor={id} className="block text-xs font-medium text-text-secondary">{sideLabels[side]}</label>
      <Button id={id} variant="outline" disabled={disabled} aria-label={`录入${sideLabels[side]}`}
        aria-pressed={recording !== null}
        className={`h-auto min-h-16 w-full whitespace-normal px-3 py-3 text-xs ${recording !== null ? "border-primary bg-primary/5" : "bg-background"}`}
        onClick={() => {
          setPicker(null); setPreview(null); setHint(""); setReady(false);
          setRecording(++recordingSequence);
        }}>
        <Keyboard size={16} />
        {recording !== null ? (preview ? chordLabel(preview, side, keys) : ready ? "请按下并松开按键…" : "准备录入…")
          : chord.key ? chordLabel(chord, side, keys) : "点击录入按键"}
      </Button>
      <div className="flex items-center justify-between gap-2">
        <Button ref={pickerButtonRef} variant="link" size="xs" disabled={disabled} aria-expanded={picker !== null} aria-label={`选择${sideLabels[side]}`}
          onClick={() => { setRecording(null); setPicker(picker ? null : chord); setSearch(""); setHint(""); }}>
          <Search size={12} />选择按键
        </Button>
        {recording !== null && <Button variant="ghost" size="xs" onClick={() => setRecording(null)}>取消录入</Button>}
      </div>
      {recording !== null && <p role="status" className="text-[11px] text-text-muted">松开后完成 · Esc 取消 · Tab 离开</p>}
      {hint && <p role="alert" className="text-[11px] text-destructive">{hint}</p>}
      {picker && (
        <section aria-label={`选择${sideLabels[side]}按键`} className="space-y-3 rounded-lg border border-border bg-background p-3">
          <Input autoFocus aria-label={`搜索${sideLabels[side]}`} placeholder="搜索名称，如 Alt、Command、F1" value={search} onChange={(event) => setSearch(event.target.value)} className="text-xs" />
          <div className="max-h-44 space-y-2 overflow-y-auto" aria-label="可选按键">
            {groups.map((group) => <div key={group}>
              <p className="mb-1 text-[10px] text-text-muted">{group}</p>
              <div className="flex flex-wrap gap-1">
                {filtered.filter((key) => key.group === group).map((key) => <Button key={key.id} size="xs" variant={picker.key === key.id ? "default" : "ghost"}
                  aria-pressed={picker.key === key.id} onClick={() => setPicker({ ...picker, key: key.id })}>{keyLabel(key, side)}</Button>)}
              </div>
            </div>)}
            {filtered.length === 0 && <p className="py-2 text-xs text-text-muted">没有匹配的按键</p>}
          </div>
          <fieldset>
            <legend className="mb-1.5 text-[11px] text-text-muted">组合修饰键（可选）</legend>
            <div className="flex flex-wrap gap-1">
              {modifiers.map((modifier) => <Button key={modifier} size="xs" variant={picker.modifiers.includes(modifier) ? "default" : "outline"}
                aria-pressed={picker.modifiers.includes(modifier)} aria-label={`${sideLabels[side]} ${modifierLabels[side][modifier]}`}
                onClick={() => setPicker({ ...picker, modifiers: picker.modifiers.includes(modifier)
                  ? picker.modifiers.filter((value) => value !== modifier) : modifiers.filter((value) => value === modifier || picker.modifiers.includes(value)) })}>
                {modifierLabels[side][modifier]}
              </Button>)}
            </div>
          </fieldset>
          <p className="break-words text-xs text-text-secondary">{picker.key ? chordLabel(picker, side, keys) : "请选择按键"}</p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="xs" onClick={() => setPicker(null)}>取消选择</Button>
            <Button size="xs" disabled={!picker.key} onClick={() => { onChange(picker); setPicker(null); }}>使用按键</Button>
          </div>
        </section>
      )}
    </div>
  );
}

export default function KeyboardSettings() {
  const [keys, setKeys] = useState<KeyOption[]>([]);
  const [mappings, setMappings] = useState<KeyMapping[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<KeyMapping | null>(null);
  const [editIndex, setEditIndex] = useState<number | null>(null);

  const load = () => {
    setLoading(true);
    setError(null);
    Promise.all([api.getConfig(), api.getKeyboardKeys()])
      .then(([config, options]) => {
        setMappings(config.keyboard_mappings);
        setKeys(options);
      })
      .catch((error: unknown) => setError(`读取按键映射失败：${String(error)}`))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, []);

  const persist = (next: KeyMapping[]) => {
    setSaving(true);
    setError(null);
    api.setConfig("keyboard_mappings", next)
      .then(() => {
        setMappings(next);
        setDraft(null);
        setEditIndex(null);
      })
      .catch((error: unknown) => setError(`保存失败：${String(error)}`))
      .finally(() => setSaving(false));
  };

  const saveDraft = () => {
    if (!draft || !draft.from.key || !draft.to.key) return;
    const duplicate = mappings.some((rule, index) => index !== editIndex
      && rule.from.key === draft.from.key
      && [...rule.from.modifiers].sort().join(",") === [...draft.from.modifiers].sort().join(","));
    if (duplicate) {
      setError("这个输入已配置映射，请编辑已有规则，或选择其他按键组合。");
      return;
    }
    const next = editIndex === null
      ? [...mappings, draft]
      : mappings.map((rule, index) => index === editIndex ? draft : rule);
    persist(next);
  };

  if (loading) {
    return <div className="flex items-center justify-center gap-2 py-20 text-sm text-text-muted"><Loader2 size={16} className="animate-spin" />加载按键映射中...</div>;
  }

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-4">
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-lg font-semibold text-text">按键映射</h1>
        <Button size="sm" disabled={saving || draft !== null || keys.length === 0} onClick={() => {
          setEditIndex(null);
          setDraft({ from: { key: "", modifiers: [] }, to: { key: "", modifiers: [] } });
          setError(null);
        }}>
          <Plus size={14} />添加映射
        </Button>
      </div>

      {error && (
        <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
          {error}
          {keys.length === 0 && <Button variant="outline" size="xs" className="ml-2" onClick={load}>重试</Button>}
        </div>
      )}

      {draft && (
        <section aria-label="映射编辑器" className="rounded-lg border border-border bg-card p-4">
          <h2 className="mb-4 text-sm font-medium text-text">{editIndex === null ? "添加映射" : "编辑映射"}</h2>
          <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-start gap-4">
            <ChordEditor side="from" chord={draft.from} keys={keys} disabled={saving} onChange={(from) => setDraft({ ...draft, from })} />
            <ArrowRight size={16} className="mt-9 text-text-muted" aria-hidden="true" />
            <ChordEditor side="to" chord={draft.to} keys={keys} disabled={saving} onChange={(to) => setDraft({ ...draft, to })} />
          </div>
          <div className="mt-4 flex justify-end gap-2 border-t border-border pt-3">
            <Button variant="outline" size="sm" disabled={saving} onClick={() => { setDraft(null); setEditIndex(null); setError(null); }}>取消</Button>
            <Button size="sm" disabled={saving || !draft.from.key || !draft.to.key} onClick={saveDraft}>
              {saving && <Loader2 size={14} className="animate-spin" />}{saving ? "保存中..." : "保存映射"}
            </Button>
          </div>
        </section>
      )}

      {mappings.length === 0 && !draft && keys.length > 0 && (
        <div className="flex flex-col items-center rounded-lg border border-dashed border-border bg-card/50 px-6 py-12 text-center">
          <Keyboard size={28} className="mb-3 text-text-muted" />
          <p className="text-sm text-text-secondary">暂无自定义映射</p>
        </div>
      )}

      {mappings.length > 0 && (
        <section aria-label="已保存的映射" className="space-y-2">
          <h2 className="text-xs text-text-muted">自定义映射 · {mappings.length}</h2>
          {mappings.map((rule, index) => (
            <div key={index} className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto] items-center gap-3 rounded-lg border border-border bg-card p-3">
              <div className="min-w-0">
                <div className="mb-1 text-[10px] text-text-muted">原按键</div>
                <div className="break-words text-xs font-medium text-text">{chordLabel(rule.from, "from", keys)}</div>
              </div>
              <ArrowRight size={14} className="text-text-muted" aria-hidden="true" />
              <div className="min-w-0">
                <div className="mb-1 text-[10px] text-text-muted">映射为</div>
                <div className="break-words text-xs font-medium text-text">{chordLabel(rule.to, "to", keys)}</div>
              </div>
              <div className="flex gap-1">
                <Button variant="ghost" size="icon-xs" aria-label={`编辑映射 ${index + 1}`} title="编辑" disabled={saving || draft !== null} onClick={() => {
                  setEditIndex(index); setDraft(rule); setError(null);
                }}><Pencil size={13} /></Button>
                <Button variant="ghost" size="icon-xs" aria-label={`删除映射 ${index + 1}`} title="删除" disabled={saving || draft !== null} onClick={() => persist(mappings.filter((_, position) => position !== index))}>
                  <Trash2 size={13} />
                </Button>
              </div>
            </div>
          ))}
        </section>
      )}
    </div>
  );
}
