// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

/** Multi-select for batch actions (ROADMAP #15): click toggles, Shift+click selects a range. */
export class Selection {
  active = $state(false);
  ids = $state<string[]>([]);
  private anchor: string | null = null;

  has(id: string): boolean {
    return this.ids.includes(id);
  }
  get size(): number {
    return this.ids.length;
  }
  /** `order` = the ids as shown, for Shift ranges. */
  toggle(id: string, order: string[], shift = false) {
    if (shift && this.anchor && order.includes(this.anchor)) {
      const [a, b] = [order.indexOf(this.anchor), order.indexOf(id)].sort((x, y) => x - y);
      const range = order.slice(a, b + 1);
      this.ids = [...new Set([...this.ids, ...range])];
    } else {
      this.ids = this.has(id) ? this.ids.filter((x) => x !== id) : [...this.ids, id];
      this.anchor = id;
    }
  }
  set(list: string[]) {
    this.ids = [...new Set(list)];
  }
  clear() {
    this.ids = [];
    this.anchor = null;
  }
  start() {
    this.active = true;
  }
  stop() {
    this.active = false;
    this.clear();
  }
  /** Keeps only ids that still exist (after a reload). */
  prune(existing: string[]) {
    const keep = new Set(existing);
    if (this.ids.some((x) => !keep.has(x))) this.ids = this.ids.filter((x) => keep.has(x));
  }
}

/** Ctrl+A selects all shown, Esc clears / leaves select mode. Ignored while typing. */
export function selectionKeys(sel: Selection, shown: () => string[]) {
  return (e: KeyboardEvent) => {
    if (!sel.active) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.closest("textarea, select, input:not([type=checkbox]):not([type=radio])") || t.isContentEditable)) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      sel.set(shown());
    } else if (e.key === "Escape") {
      if (sel.size) sel.clear();
      else sel.stop();
    }
  };
}

/** "A, B, C, D, E +3 more" */
export function nameList(names: string[], max = 5): string {
  return names.length <= max ? names.join(", ") : `${names.slice(0, max).join(", ")} +${names.length - max} more`;
}
