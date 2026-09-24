import { useEffect } from 'react';
import type { Settings } from '../types';

/**
 * Root font size for each scale step.
 *
 * Three steps rather than a free number: the window height is fixed and the toolbar is
 * a fixed height strip, so there is a range that looks right and a range that crowds.
 * The largest is 18 rather than 20 for the same reason.
 */
const SCALE_PX = {
  small: 14,
  default: 16,
  large: 18,
} as const;

/** Root font size for a scale value coming from settings, which is an open string. */
function scaleToPx(scale: string | undefined): number {
  if (scale === 'small' || scale === 'large') {
    return SCALE_PX[scale];
  }
  return SCALE_PX.default;
}

/**
 * Applies the typography settings as CSS custom properties on the root element.
 *
 * Variables rather than inline styles, so one change re-themes everything: nearly all
 * text is sized in rem, which follows the root font size.
 *
 * An unset family is *removed* rather than set to an empty string. An empty custom
 * property makes the declaration invalid and the whole thing is dropped, whereas an
 * unset one lets `var(--x, fallback)` use its fallback.
 */
export function useTypography(settings: Settings | null) {
  const scale = settings?.font_scale ?? 'default';
  const uiFamily = settings?.ui_font_family?.trim() ?? '';
  const uiWeight = settings?.ui_font_weight ?? '400';
  const contentFamily = settings?.content_font_family?.trim() ?? '';
  const contentWeight = settings?.content_font_weight ?? '400';

  useEffect(() => {
    const root = document.documentElement;

    root.style.setProperty('--font-scale', `${scaleToPx(scale)}px`);
    root.style.setProperty('--ui-font-weight', uiWeight);
    root.style.setProperty('--content-font-weight', contentWeight);

    // Quoted so a family name containing a space is still one family.
    if (uiFamily) {
      root.style.setProperty('--ui-font-family', `"${uiFamily}"`);
    } else {
      root.style.removeProperty('--ui-font-family');
    }

    if (contentFamily) {
      root.style.setProperty('--content-font-family', `"${contentFamily}"`);
    } else {
      root.style.removeProperty('--content-font-family');
    }
  }, [scale, uiFamily, uiWeight, contentFamily, contentWeight]);
}
