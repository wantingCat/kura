export interface LangOption {
  value: string;
  label: string;
}

export const AUDIO_LANG_OPTIONS: LangOption[] = [
  { value: "jpn", label: "Japanese" },
  { value: "eng", label: "English" },
  { value: "ger", label: "German" },
  { value: "fre", label: "French" },
  { value: "spa", label: "Spanish" },
  { value: "ita", label: "Italian" },
  { value: "por", label: "Portuguese" },
  { value: "chi", label: "Chinese" },
  { value: "kor", label: "Korean" },
  { value: "rus", label: "Russian" },
  { value: "any", label: "Any / Default" },
];

export const SUB_LANG_OPTIONS: LangOption[] = [
  { value: "jpn", label: "Japanese" },
  { value: "eng", label: "English" },
  { value: "ger", label: "German" },
  { value: "fre", label: "French" },
  { value: "spa", label: "Spanish" },
  { value: "ita", label: "Italian" },
  { value: "por", label: "Portuguese" },
  { value: "chi", label: "Chinese" },
  { value: "kor", label: "Korean" },
  { value: "rus", label: "Russian" },
  { value: "off", label: "Off (No subtitles)" },
  { value: "any", label: "Any / Default" },
];

export const SUB_FALLBACK_OPTIONS: LangOption[] = [
  { value: "none", label: "None (No fallback)" },
  { value: "eng", label: "English" },
  { value: "jpn", label: "Japanese" },
  { value: "ger", label: "German" },
  { value: "fre", label: "French" },
  { value: "spa", label: "Spanish" },
  { value: "ita", label: "Italian" },
  { value: "por", label: "Portuguese" },
  { value: "chi", label: "Chinese" },
  { value: "kor", label: "Korean" },
  { value: "rus", label: "Russian" },
];

export interface TrackPreset {
  id: string;
  name: string;
  blurb: string;
  audio: string;
  sub: string;
  fallback: string;
}

export const TRACK_PRESETS: TrackPreset[] = [
  {
    id: "jp-jp-en",
    name: "Japanese + Japanese Subs (fallback English)",
    blurb: "Ideal for immersion/learning: Japanese subs first, English subs if missing.",
    audio: "jpn",
    sub: "jpn",
    fallback: "eng",
  },
  {
    id: "jp-en",
    name: "Japanese + English Subs",
    blurb: "Classic subbed experience: original Japanese voice acting with English subs.",
    audio: "jpn",
    sub: "eng",
    fallback: "none",
  },
  {
    id: "en-off",
    name: "English Dub (Subs Off)",
    blurb: "English voice acting with subtitles turned off.",
    audio: "eng",
    sub: "off",
    fallback: "none",
  },
  {
    id: "en-en",
    name: "English Dub + English Subs",
    blurb: "English audio with English subtitles / captions.",
    audio: "eng",
    sub: "eng",
    fallback: "none",
  },
];

export function langLabel(options: LangOption[], val: string): string {
  const found = options.find((o) => o.value === val);
  return found ? found.label : val;
}

export function describeTrackSummary(audio: string, sub: string, fallback: string): string {
  const a = langLabel(AUDIO_LANG_OPTIONS, audio);
  if (sub === "off") {
    return `${a} Audio · Subs Off`;
  }
  const s = langLabel(SUB_LANG_OPTIONS, sub);
  if (!fallback || fallback === "none" || fallback === sub) {
    return `${a} Audio · ${s} Subs`;
  }
  const f = langLabel(SUB_FALLBACK_OPTIONS, fallback);
  return `${a} Audio · ${s} Subs (${f} fallback)`;
}

export function matchingPresetId(audio: string, sub: string, fallback: string): string {
  const normFallback = !fallback || fallback === "" ? "none" : fallback;
  for (const p of TRACK_PRESETS) {
    if (p.audio === audio && p.sub === sub && (p.fallback === normFallback || (p.fallback === "none" && normFallback === "none"))) {
      return p.id;
    }
  }
  return "custom";
}
