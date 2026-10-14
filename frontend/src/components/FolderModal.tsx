import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FOLDER_DOT_COLORS } from '../lib/folderColors';

interface FolderModalProps {
  isOpen: boolean;
  mode: 'create' | 'rename';
  initialName: string;
  onClose: () => void;
  onSubmit: (name: string, color?: string | null) => void;
  /**
   * Overrides for the folder wording, so the same dialog can name other things.
   */
  title?: string;
  placeholder?: string;
  /**
   * Whether submitting an empty field is allowed. Folders need a name, so it is off by
   * default; a clip title can be cleared, which is what empty means there.
   */
  allowEmpty?: boolean;
  /**
   * Whether to offer a colour. The clip title reuses this dialog, and a title has none.
   */
  showColor?: boolean;
  initialColor?: string | null;
}

export function FolderModal({
  isOpen,
  mode,
  initialName,
  onClose,
  onSubmit,
  title,
  placeholder,
  allowEmpty = false,
  showColor = false,
  initialColor = null,
}: FolderModalProps) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [color, setColor] = useState<string | null>(initialColor);

  useEffect(() => {
    if (isOpen) {
      setIsSubmitting(false); // Reset on open
      setColor(initialColor);
      if (inputRef.current) {
        // slight delay to ensure render
        setTimeout(() => inputRef.current?.focus(), 50);
        // If rename, select all text
        if (mode === 'rename') {
          setTimeout(() => inputRef.current?.select(), 50);
        }
      }
    }
  }, [isOpen, mode, initialColor]);

  if (!isOpen) return null;

  const handleSubmit = async () => {
    if (isSubmitting) return;
    const val = inputRef.current?.value.trim() ?? '';
    if (!val && !allowEmpty) return;

    setIsSubmitting(true);
    await onSubmit(val, color);
    setIsSubmitting(false);
  };

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
      <div className="w-80 rounded-2xl border border-border bg-card p-6 shadow-2xl">
        <h3 className="mb-4 text-lg font-semibold text-foreground">
          {title ?? (mode === 'create' ? t('folders.createFolder') : t('folders.renameFolder'))}
        </h3>
        <input
          ref={inputRef}
          type="text"
          placeholder={placeholder ?? t('folders.folderNamePlaceholder')}
          defaultValue={initialName}
          className="mb-4 w-full rounded-md border border-input bg-input px-3 py-2 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              handleSubmit();
            } else if (e.key === 'Escape') {
              onClose();
            }
          }}
        />
        {showColor && (
          <div className="mb-4 flex flex-wrap gap-1.5">
            {FOLDER_DOT_COLORS.map((option) => (
              <button
                key={option}
                type="button"
                aria-label={option}
                onClick={() => setColor(option)}
                style={{ backgroundColor: option }}
                className={
                  color === option
                    ? 'h-5 w-5 rounded-full ring-2 ring-foreground ring-offset-2 ring-offset-card'
                    : 'h-5 w-5 rounded-full ring-1 ring-border transition-transform hover:scale-110'
                }
              />
            ))}
          </div>
        )}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={isSubmitting}
            className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:bg-secondary hover:text-foreground disabled:opacity-50"
          >
            {t('common.cancel')}
          </button>
          <button
            onClick={handleSubmit}
            disabled={isSubmitting}
            className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {isSubmitting
              ? t('common.loading')
              : mode === 'create'
                ? t('common.create')
                : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
