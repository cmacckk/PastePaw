import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

import en from './locales/en.json';
import zh from './locales/zh.json';

const resources = {
  en: {
    translation: en,
  },
  zh: {
    translation: zh,
  },
};

/** Where the chosen language is remembered. */
export const LANGUAGE_STORAGE_KEY = 'vigilclip_language';
/** The app used to store it under a key named after the name it had then. */
const LEGACY_LANGUAGE_STORAGE_KEY = 'pastepaw_language';

// Carry a choice made before the rename across, so changing the key does not send
// someone who picked a language back to the default.
const rememberedLanguage =
  localStorage.getItem(LANGUAGE_STORAGE_KEY) ?? localStorage.getItem(LEGACY_LANGUAGE_STORAGE_KEY);
if (rememberedLanguage) {
  localStorage.setItem(LANGUAGE_STORAGE_KEY, rememberedLanguage);
  localStorage.removeItem(LEGACY_LANGUAGE_STORAGE_KEY);
}

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    // Only these two are shipped, so a browser asking for anything else gets Chinese.
    supportedLngs: ['zh', 'en'],
    fallbackLng: 'zh',
    interpolation: {
      escapeValue: false, // React already escapes
    },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      lookupLocalStorage: LANGUAGE_STORAGE_KEY,
    },
  });

export default i18n;
