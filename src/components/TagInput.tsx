import { useState } from "react";
import { X } from "lucide-react";

/** Comma/Enter separated tags with removable chips and optional suggestions. */
export default function TagInput({
  tags, onChange, suggestions, placeholder, label,
}: { tags: string[]; onChange: (tags: string[]) => void; suggestions: string[]; placeholder: string; label: string }) {
  const [draft, setDraft] = useState("");
  const listId = `tags-${label.replace(/\W+/g, "-").toLowerCase()}`;

  function commit(raw: string) {
    const added = raw.split(",").map((t) => t.trim()).filter(Boolean);
    if (!added.length) { setDraft(""); return; }
    const known = new Set(tags.map((t) => t.toLowerCase()));
    const next = [...tags];
    for (const tag of added) if (!known.has(tag.toLowerCase())) { known.add(tag.toLowerCase()); next.push(tag); }
    onChange(next);
    setDraft("");
  }

  return (
    <div className="taginput" onClick={(e) => (e.currentTarget.querySelector("input") as HTMLInputElement | null)?.focus()}>
      <span className="taglabel">{label}</span>
      {tags.map((tag) => (
        <span key={tag} className="chip accent tag">
          {tag}
          <button aria-label={`Remove ${tag}`} onClick={() => onChange(tags.filter((t) => t !== tag))}><X size={11} /></button>
        </span>
      ))}
      <input
        list={listId} value={draft} placeholder={tags.length ? "" : placeholder} spellCheck={false} aria-label={label}
        onChange={(e) => {
          const v = e.target.value;
          if (v.includes(",")) commit(v); else setDraft(v);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); commit(draft); }
          else if (e.key === "Backspace" && !draft && tags.length) onChange(tags.slice(0, -1));
        }}
        onBlur={() => commit(draft)}
      />
      <datalist id={listId}>{suggestions.map((s) => <option key={s} value={s} />)}</datalist>
    </div>
  );
}
