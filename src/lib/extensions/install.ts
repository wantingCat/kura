import {
  api,
  type InstalledExtension,
  type MarketplaceExtension,
  type TorrentExtension,
} from "../api";
import { app } from "../app.svelte";
import { evictExtensionWorker } from "./host";
import { invalidateOnlinestreamCache } from "../onlinestream.svelte";

export type ExtensionInstallResult =
  | { kind: "seanime_installed"; extension: InstalledExtension }
  | { kind: "marketplace_catalog"; url: string; items: MarketplaceExtension[] }
  | { kind: "native_torrent_sources"; extensions: TorrentExtension[] };

export interface InstallExtensionOptions {
  /**
   * When `true` (Settings page), multi-extension repositories and marketplace JSON catalogs
   * return `{ kind: "marketplace_catalog" }` so the user can browse and install individual items.
   * When `false` (quick-install inside `TorrentPickerDialog` or `WebStreamDialog`), a repository
   * or catalog with 1 matching extension auto-installs immediately, or returns the filtered
   * catalog items for 1-click selection.
   */
  allowCatalogBrowse?: boolean;
  /** Optional provider type preference for quick-install dialogs. */
  preferredExtType?: "onlinestream-provider" | "anime-torrent-provider";
  /** Whether to fall back to `api.importTorrentExtension` for RSS/SubsPlease/SeaDex/Hayase feeds. */
  allowNativeTorrentFallback?: boolean;
}

export function cleanExtensionError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  return raw.replace(/^(?:Error:\s*)?[A-Z_]{4,}:\s*/, "").trim() || "Extension installation failed.";
}

function isGitHubRepoUrl(val: string): boolean {
  if (
    val.endsWith(".json") ||
    val.endsWith(".xml") ||
    val.endsWith(".rss") ||
    val.includes("/blob/") ||
    val.includes("/raw/")
  ) {
    return false;
  }
  return (
    val.startsWith("gh:") ||
    /^https?:\/\/github\.com\/[^/]+\/[^/]+(?:\/tree\/[^?#]+)?\/?$/i.test(val)
  );
}

function looksLikeSeanimeManifest(val: string): boolean {
  return (
    val.startsWith("{") ||
    val.endsWith(".json") ||
    val.includes("onlinestream") ||
    val.includes("manifest") ||
    val.includes("/blob/") ||
    val.includes("raw.githubusercontent.com")
  );
}

function isExplicitSeanimeManifestError(errStr: string): boolean {
  return (
    errStr.includes("UNSUPPORTED_SEANIME_TYPE") ||
    errStr.includes("SEANIME_MANIFEST_ERR") ||
    errStr.includes("Unsupported extension type") ||
    errStr.includes("did not define a Provider")
  );
}

async function activateStreamingAfterInstall(installedKind: "web" | "torrent") {
  await app.refreshExtensions();
  if (installedKind === "web" && app.hasEnabledWebStreamProvider && !app.hasEnabledTorrentProvider) {
    await app.setStreamSource("web");
  } else if (installedKind === "torrent" && app.hasEnabledTorrentProvider && !app.hasEnabledWebStreamProvider) {
    await app.setStreamSource("torrent");
  }
  if (app.appMode === "local") {
    await app.setAppMode("streaming");
  }
}

async function resolveCatalogResult(
  url: string,
  items: MarketplaceExtension[],
  opts: InstallExtensionOptions,
): Promise<ExtensionInstallResult> {
  await app.setExtensionMarketplaceUrl(url);

  if (opts.allowCatalogBrowse) {
    return { kind: "marketplace_catalog", url, items };
  }

  const matching = opts.preferredExtType
    ? items.filter((m) => m.type === opts.preferredExtType && Boolean(m.manifestUri))
    : items.filter((m) => Boolean(m.manifestUri));

  if (matching.length === 1) {
    const extension = await api.installExtension(matching[0].manifestUri);
    evictExtensionWorker(extension.id);
    invalidateOnlinestreamCache();
    await activateStreamingAfterInstall(
      extension.extType === "onlinestream-provider" ? "web" : "torrent",
    );
    return { kind: "seanime_installed", extension };
  }

  if (matching.length > 1) {
    return { kind: "marketplace_catalog", url, items: matching };
  }

  if (opts.preferredExtType && items.length > 0) {
    const label =
      opts.preferredExtType === "onlinestream-provider"
        ? "web streaming (onlinestream-provider)"
        : "torrent (anime-torrent-provider)";
    throw new Error(
      `Found ${items.length} extension(s) in that source, but none are ${label} extensions.`,
    );
  }

  return { kind: "marketplace_catalog", url, items };
}

/**
 * Unified extension installer shared by `Settings`, `TorrentPickerDialog`, and `WebStreamDialog`.
 * Handles GitHub repo URLs (`https://github.com/owner/repo`, `gh:owner/repo`), GitHub `/blob/` URLs,
 * raw Seanime manifest JSON/URLs, Seanime Marketplace catalogs, Hayase `index.json`, and native RSS/JSON feeds.
 */
export async function installUnifiedExtension(
  rawInput: string,
  opts: InstallExtensionOptions = {},
): Promise<ExtensionInstallResult> {
  const val = rawInput.trim();
  if (!val) {
    throw new Error("Paste an extension manifest URL, GitHub repository URL, or feed URL first.");
  }

  const allowNative = opts.allowNativeTorrentFallback !== false;

  // 1. GitHub repository URL (`https://github.com/owner/repo` or `gh:owner/repo`)
  if (isGitHubRepoUrl(val)) {
    let repoErr: unknown = null;
    const items = await api.fetchMarketplaceExtensions(val).catch((e) => {
      repoErr = e;
      return [] as MarketplaceExtension[];
    });
    if (items.length > 0) {
      return resolveCatalogResult(val, items, opts);
    }
    if (!allowNative) {
      throw new Error(
        repoErr
          ? cleanExtensionError(repoErr)
          : "No compatible Seanime anime extensions were found in that GitHub repository.",
      );
    }
  }

  // 2. Seanime manifest JSON, `.json` URL, `/blob/` URL, or Marketplace JSON catalog
  if (looksLikeSeanimeManifest(val) || !allowNative) {
    try {
      const extension = await api.installExtension(val);
      if (opts.preferredExtType && extension.extType !== opts.preferredExtType && !allowNative) {
        // Still installed cleanly, surface it to caller
      }
      evictExtensionWorker(extension.id);
      invalidateOnlinestreamCache();
      await activateStreamingAfterInstall(
        extension.extType === "onlinestream-provider" ? "web" : "torrent",
      );
      return { kind: "seanime_installed", extension };
    } catch (seanimeErr) {
      const errStr = String(seanimeErr);
      if (isExplicitSeanimeManifestError(errStr)) {
        throw new Error(cleanExtensionError(seanimeErr));
      }
      // Check if it's a Seanime Marketplace catalog JSON array/object
      if (val.startsWith("http://") || val.startsWith("https://")) {
        const items = await api.fetchMarketplaceExtensions(val).catch(() => []);
        if (items.length > 0) {
          return resolveCatalogResult(val, items, opts);
        }
      }
      if (!allowNative) {
        throw new Error(cleanExtensionError(seanimeErr));
      }
    }
  }

  // 3. Native Rust torrent feed / Hayase index.json / SubsPlease / SeaDex / AnimeTosho
  try {
    const extensions = await api.importTorrentExtension(val);
    await activateStreamingAfterInstall("torrent");
    return { kind: "native_torrent_sources", extensions };
  } catch (e) {
    throw new Error(cleanExtensionError(e));
  }
}
