import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import zh from "./locales/zh.json";

type Language = "zh";

const getInitialLanguage = (): Language => "zh";

const resources = {
  zh: {
    translation: zh,
  },
};

i18n.use(initReactI18next).init({
  resources,
  lng: getInitialLanguage(), // 仅支持简体中文
  fallbackLng: "zh", // 缺失 key 时回退到 zh 自身

  interpolation: {
    escapeValue: false, // React 已经默认转义
  },

  // 开发模式下显示调试信息
  debug: false,
});

export default i18n;
