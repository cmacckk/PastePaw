import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface EditClipModalProps {
  isOpen: boolean;
  initialContent: string;
  onClose: () => void;
  onSubmit: (content: string) => void;
}

/**
 * Edits a clip's text.
 *
 * A textarea rather than the single line dialog used for names: the reason to edit a
 * clip is usually that it is multi-line, and Enter has to insert a newline there
 * instead of submitting. Ctrl or Cmd with Enter submits.
 *
 * Sized to the window rather than to a fixed height. This dialog renders inside the
 * main window, which is only a couple of hundred pixels tall, so a fixed-height box
 * would be clipped.
 */
export function EditClipModal({ isOpen, initialContent, onClose, onSubmit }: EditClipModalProps) {
  const { t } = useTranslation();
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setIsSubmitting(false);
      // Slight delay so the element exists before it is focused.
      setTimeout(() => {
        const area = areaRef.current;
        if (area) {
          area.focus();
          area.setSelectionRange(0, 0);
          area.scrollTop = 0;
        }
      }, 50);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async () => {
    if (isSubmitting) return;
    // Blank is refused rather than treated as a deletion: removing a clip is what the
    // delete action is for.
    const value = areaRef.current?.value.trim() ?? '';
    if (!value) return;

    setIsSubmitting(true);
    await onSubmit(value);
    setIsSubmitting(false);
  };

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/50 p-2 backdrop-blur-sm">
      <div className="flex max-h-full w-[32rem] max-w-full flex-col rounded-2xl border border-border bg-card p-4 shadow-2xl">
        <h3 className="mb-2 text-sm font-semibold text-foreground">{t('clipList.editClip')}</h3>
        <textarea
          ref={areaRef}
          defaultValue={initialContent}
          className="clip-content-font mb-3 min-h-[6rem] w-full flex-1 resize-none rounded-md border border-input bg-input px-3 py-2 text-xs text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              onClose();
            } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              handleSubmit();
            }
          }}
        />
        <div className="flex flex-shrink-0 justify-end gap-2">
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
            {isSubmitting ? t('common.loading') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
