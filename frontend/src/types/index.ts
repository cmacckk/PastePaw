export interface ClipboardItem {
  id: string;
  clip_type: string;
  content: string;
  preview: string;
  folder_id: string | null;
  created_at: string;
  source_app: string | null;
  source_icon: string | null;
  metadata: string | null;
  /** A name the user gave this clip. Null means fall back to the source application. */
  title?: string | null;
}

export interface FolderItem {
  id: string;
  name: string;
  icon: string | null;
  color: string | null;
  is_system: boolean;
  item_count: number;
}

/** What an import did. Mirrors `ImportReport` in src-tauri/src/export.rs. */
export interface ImportReport {
  folders_created: number;
  folders_reused: number;
  clips_imported: number;
  clips_skipped: number;
  images_restored: number;
  images_missing: number;
}

/** A source application that has clips, with how many. Mirrors the Rust type. */
export interface SourceAppCount {
  name: string;
  count: number;
}

export interface Settings {
  max_items: number;
  auto_delete_days: number;
  startup_with_windows: boolean;
  show_in_taskbar: boolean;
  hotkey: string;
  theme: string;
  language?: string;
  mica_effect?: string;
  round_corners?: boolean;
  float_above_taskbar?: boolean;
  card_size?: 'large' | 'medium';
  /** Empty means keep the stylesheet's font. */
  ui_font_family?: string;
  ui_font_weight?: string;
  content_font_family?: string;
  content_font_weight?: string;
  font_scale?: 'small' | 'default' | 'large';
  auto_paste: boolean;
  paste_method?: 'shift_insert' | 'ctrl_v';
  ignore_ghost_clips: boolean;
}

export type ClipType = 'text' | 'image' | 'html' | 'rtf' | 'file' | 'url';

export const CLIP_TYPE_LABELS: Record<ClipType, string> = {
  text: 'Text',
  image: 'Image',
  html: 'HTML',
  rtf: 'Rich Text',
  file: 'File',
  url: 'URL',
};

export const CLIP_TYPE_ICONS: Record<ClipType, string> = {
  text: 'FileText',
  image: 'Image',
  html: 'Code',
  rtf: 'Type',
  file: 'File',
  url: 'Link',
};
