import { FolderItem, SourceAppCount } from '../types';
import { Search, Plus, MoreHorizontal, X, ArrowUpCircle, AlignLeft, Image, File } from 'lucide-react';
import { clsx } from 'clsx';
import { useTranslation } from 'react-i18next';
import { Select } from './ui/Select';

interface ControlBarProps {
  folders: FolderItem[];
  selectedFolder: string | null;
  onSelectFolder: (folderId: string | null) => void;
  onSearchClick: () => void;
  onAddClick: () => void;
  onMoreClick: () => void;
  showSearch: boolean;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  onMoveClip: (clipId: string, folderId: string | null) => void;
  isDragging: boolean;
  dragTargetFolderId: string | null;
  onDragHover: (folderId: string | null) => void;
  onDragLeave: () => void;
  totalClipCount: number;
  /** Clip types currently filtered in. Empty means every type is shown. */
  selectedTypes: string[];
  onToggleType: (type: string) => void;
  /** Source applications present in the history, for the filter's choices. */
  sourceApps: SourceAppCount[];
  selectedSourceApp: string | null;
  onSelectSourceApp: (name: string | null) => void;
  onFolderContextMenu?: (e: React.MouseEvent, folderId: string) => void;
  style?: React.CSSProperties;
  updateAvailable?: import('../hooks/useAutoUpdater').UpdateInfo | null;
}

/**
 * The clip types capture actually produces, in the order they are shown.
 *
 * There is deliberately no link or colour entry: those are not clip types, and a
 * filter for them would promise something capture never produces.
 */
const CLIP_TYPE_FILTERS = [
  { type: 'text', Icon: AlignLeft, labelKey: 'clipTypes.text' },
  { type: 'image', Icon: Image, labelKey: 'clipTypes.image' },
  { type: 'file', Icon: File, labelKey: 'clipTypes.file' },
] as const;

/// Colours for the folder dots. A folder keeps the colour it was given; the rest fall
/// back to a stable colour derived from the name, so a folder never changes colour.
const FOLDER_DOT_COLORS = [
  '#ef4444',
  '#f97316',
  '#eab308',
  '#22c55e',
  '#14b8a6',
  '#06b6d4',
  '#3b82f6',
  '#8b5cf6',
  '#ec4899',
  '#64748b',
];

function folderDotColor(name: string, color?: string | null) {
  if (color) {
    return color;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i += 1) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
  }
  return FOLDER_DOT_COLORS[Math.abs(hash) % FOLDER_DOT_COLORS.length];
}

export function ControlBar({
  folders,
  selectedFolder,
  onSelectFolder,
  onSearchClick,
  onAddClick,
  onMoreClick,
  showSearch,
  searchQuery,
  onSearchChange,
  isDragging,
  dragTargetFolderId,
  onDragHover,
  onDragLeave,
  totalClipCount,
  selectedTypes,
  onToggleType,
  sourceApps,
  selectedSourceApp,
  onSelectSourceApp,
  onFolderContextMenu,
  style,
  updateAvailable,
}: ControlBarProps) {
  const { t } = useTranslation();

  const allCategories = [
    { id: null, name: t('folders.all'), count: totalClipCount },
    ...folders.map((f) => ({ ...f, count: f.item_count })),
  ];

  const handleMouseEnter = (folderId: string | null) => {
    if (isDragging) {
      onDragHover(folderId);
    }
  };

  const handleMouseLeave = () => {
    onDragLeave();
  };


  return (
    <div data-el="control-bar" className="drag-area flex items-end gap-4 px-6 pb-0" style={style}>
      {/* Search Toggle / Input */}
      <div
        data-el="search-toggle"
        className={clsx(
          'no-drag flex items-center transition-all duration-300',
          showSearch ? 'w-[300px]' : 'w-10'
        )}
      >
        {/** Search Render Code Omitted here for brevity, referencing original structure **/}
        {showSearch ? (
          <div
            data-el="search-input-wrapper"
            className="animate-in fade-in slide-in-from-left-2 flex w-full items-center gap-2 rounded-full border border-border bg-input px-3 py-1.5 duration-300"
          >
            <Search size={18} className="text-blue-400" />
            <input
              data-el="search-input"
              autoFocus
              type="text"
              value={searchQuery}
              onChange={(e) => onSearchChange(e.target.value)}
              placeholder={t('common.search')}
              className="flex-1 border-none bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  onSearchClick();
                }
              }}
            />
            <button
              data-el="search-close-btn"
              onClick={onSearchClick}
              className="rounded-full p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              <X size={16} />
            </button>
          </div>
        ) : (
          <button
            data-el="search-btn"
            onClick={onSearchClick}
            className="rounded-lg p-2 text-blue-400 transition-colors hover:bg-blue-500/10"
          >
            <Search size={20} />
          </button>
        )}
      </div>

      {/* Category Pills (Always visible) */}
      <div
        data-el="folder-pills"
        className="no-scrollbar mask-gradient-right flex flex-1 items-center gap-2 overflow-x-auto p-1"
        style={{ WebkitAppRegion: 'no-drag' } as any}
      >
        {allCategories.map((cat) => {
          const isActive = selectedFolder === cat.id;

          return (
            <button
              key={cat.id ?? 'all'}
              data-el="folder-pill"
              data-folder-id={cat.id ?? 'all'}
              onClick={() => onSelectFolder(cat.id)}
              onMouseEnter={() => handleMouseEnter(cat.id)}
              onMouseLeave={handleMouseLeave}
              onMouseUp={() => {
                // MouseUp logic is handled globally
              }}
              onContextMenu={(e) => {
                if (onFolderContextMenu && cat.id) {
                  onFolderContextMenu(e, cat.id);
                }
              }}
              style={{ WebkitAppRegion: 'no-drag' } as any}
              className={clsx(
                'flex items-center gap-2 whitespace-nowrap rounded-full px-3 py-1.5 text-sm font-medium transition-colors',
                isActive
                  ? 'bg-accent text-foreground'
                  : 'text-muted-foreground hover:bg-accent hover:text-foreground',
                isDragging && cat.id === dragTargetFolderId && 'ring-2 ring-primary'
              )}
            >
              {/* The coloured dot is what tells folders apart, as Paste does. */}
              <span
                data-el="folder-dot"
                aria-hidden="true"
                className="h-2 w-2 shrink-0 rounded-full"
                style={{
                  backgroundColor: folderDotColor(
                    cat.name,
                    'color' in cat ? (cat.color as string | null) : null
                  ),
                }}
              />
              {/* The synthetic "All Clips" pill carries no `is_system`, hence the `in`. */}
              <span>{'is_system' in cat && cat.is_system ? t('folders.pinned') : cat.name}</span>
              {/* Show count badge if defined and > 0 */}
              {cat.count !== undefined && cat.count > 0 && (
                <span className="text-[0.625rem] opacity-70">{cat.count}</span>
              )}
            </button>
          );
        })}

        {/*
          The type filter shares the folder row instead of taking a row of its own:
          the window is 266 px tall, so a second row would mean resizing it, and the
          sizing path is exactly what issue #12 was about. The divider is what keeps
          the two filter dimensions apart.
        */}
        <span
          data-el="filter-divider"
          aria-hidden="true"
          className="mx-1 h-4 w-px shrink-0 self-center bg-border"
        />

        {CLIP_TYPE_FILTERS.map(({ type, Icon, labelKey }) => {
          const isActive = selectedTypes.includes(type);
          return (
            <button
              key={type}
              data-el="type-filter"
              data-clip-type={type}
              type="button"
              title={t(labelKey)}
              aria-pressed={isActive}
              onClick={() => onToggleType(type)}
              className={clsx(
                'shrink-0 rounded-full px-2.5 py-1.5 transition-all',
                isActive
                  ? 'bg-primary/25 text-foreground ring-1 ring-primary/50'
                  : 'text-muted-foreground hover:bg-accent hover:text-foreground'
              )}
            >
              <Icon size={14} />
            </button>
          );
        })}
      </div>

      {/*
        The source application filter sits outside the pill row on purpose: that row
        scrolls horizontally, and an absolutely positioned dropdown inside it would be
        clipped away. Hidden until some clip actually carries a source application.
      */}
      {sourceApps.length > 0 && (
        <div
          data-el="source-app-filter"
          className="shrink-0"
          style={{ WebkitAppRegion: 'no-drag' } as any}
        >
          <Select
            value={selectedSourceApp ?? ''}
            onChange={(value) => onSelectSourceApp(value === '' ? null : value)}
            options={[
              { value: '', label: t('sourceApps.all') },
              ...sourceApps.map((app) => ({
                value: app.name,
                label: `${app.name} (${app.count})`,
              })),
            ]}
            className="w-36"
          />
        </div>
      )}

      {/* Actions */}
      <div
        data-el="control-bar-actions"
        className="flex flex-shrink-0 items-center gap-2"
        style={{ WebkitAppRegion: 'no-drag' } as any}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        <button
          data-el="add-folder-btn"
          onClick={onAddClick}
          className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          title={t('folders.add')}
        >
          <Plus size={20} />
        </button>
        {updateAvailable && (
          <button
            onClick={onMoreClick}
            className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground relative group"
            title={t('settings.updateAvailable', { version: updateAvailable.version })}
          >
            <div className="absolute top-[6px] right-[6px] h-2 w-2 rounded-full border-2 border-background bg-blue-500 z-10 animate-pulse"></div>
            <ArrowUpCircle size={20} />
          </button>
        )}
        <button
          data-el="settings-btn"
          onClick={onMoreClick}
          className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <MoreHorizontal size={20} />
        </button>
      </div>
    </div>
  );
}
