const hasTauri = typeof window !== "undefined" && !!(window.__TAURI__ && window.__TAURI__.core);
const appWindow = (typeof window !== "undefined" && window.__TAURI__ && window.__TAURI__.window && typeof window.__TAURI__.window.getCurrentWindow === "function")
    ? window.__TAURI__.window.getCurrentWindow()
    : {
        minimize: () => {},
        toggleMaximize: () => {},
        close: () => window.close(),
    };

async function invoke(command, args = {}) {
    if (hasTauri) {
        return window.__TAURI__.core.invoke(command, args);
    }
    switch (command) {
        case "get_dashboard_status": {
            const res = await fetch("/api/dashboard");
            return await res.json();
        }
        case "get_locations": {
            const res = await fetch("/api/locations");
            return await res.json();
        }
        case "get_snapshots": {
            const res = await fetch("/api/snapshots");
            return await res.json();
        }
        case "get_safety_backups": {
            const res = await fetch("/api/safety-backups");
            return await res.json();
        }
        case "get_reconciliation_status": {
            const res = await fetch("/api/reconciliation");
            return await res.json();
        }
        case "trigger_reconcile": {
            const res = await fetch("/api/reconcile-now", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            const data = await res.json();
            return `${data.files_changed || 0} arquivo(s) modificado(s)`;
        }
        case "restore_snapshot": {
            const res = await fetch("/api/restore", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    snapshot_id: args.snapshotId || args.snapshot_id,
                }),
            });
            const data = await res.json();
            if (!res.ok || data.success === false) {
                throw new Error(data.error || "Falha na restauração do snapshot");
            }
            return data;
        }
        case "get_setup_status": {
            const res = await fetch("/api/setup/status");
            return await res.json();
        }
        case "get_available_libraries": {
            const res = await fetch("/api/setup/libraries");
            return await res.json();
        }
        case "run_setup_discovery": {
            const selected = args.selectedLibraries || args.selected_libraries;
            const res = await fetch("/api/setup/scan", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({ selected_libraries: selected }),
            });
            const data = await res.json();
            return data.games || [];
        }
        case "get_cloud_folders": {
            const res = await fetch("/api/setup/cloud-folders");
            return await res.json();
        }
        case "save_setup_cloud": {
            const res = await fetch("/api/setup/login-cloud", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    provider: args.provider,
                    local_sync_path: args.localSyncPath || args.local_sync_path,
                }),
            });
            return await res.json();
        }
        case "complete_onboarding": {
            const payload = args.req || args;
            const res = await fetch("/api/setup/onboarding", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify(payload),
            });
            const data = await res.json();
            if (!res.ok || data.success === false) {
                throw new Error(data.error || "Falha ao registrar onboarding");
            }
            return data;
        }
        case "skip_onboarding": {
            const res = await fetch("/api/setup/skip", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
            });
            const data = await res.json();
            if (!res.ok) {
                throw new Error(data.error || "Falha ao registrar conclusão do onboarding");
            }
            return data;
        }
        case "complete_setup": {
            const res = await fetch("/api/setup/complete", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    provider: args.provider,
                    autostart: args.autostart,
                    notifications: args.notifications,
                    local_sync_path: args.localSyncPath || args.local_sync_path,
                }),
            });
            return await res.json();
        }
        case "add_game": {
            const res = await fetch("/api/games/add", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    name: args.name,
                    path: args.path,
                }),
            });
            return await res.json();
        }
        case "open_folder": {
            const res = await fetch("/api/open-folder", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    path: args.path,
                }),
            });
            return await res.json();
        }
        case "pick_folder": {
            const res = await fetch("/api/setup/pick-folder", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            const data = await res.json();
            return data.path || null;
        }
        case "get_cloud_config": {
            const res = await fetch("/api/cloud/config");
            return await res.json();
        }
        case "update_cloud_config": {
            const res = await fetch("/api/cloud/update", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    provider: args.provider,
                    local_sync_path: args.localSyncPath || args.local_sync_path,
                    enabled: args.enabled,
                }),
            });
            return await res.json();
        }
        case "optimize_database": {
            const res = await fetch("/api/maintenance/optimize", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            return await res.json();
        }
        case "export_backup_zip": {
            const res = await fetch("/api/maintenance/export", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            return await res.json();
        }
        case "get_pending_remappings": {
            const res = await fetch("/api/remapping/pending");
            return await res.json();
        }
        case "validate_remapping_path": {
            const res = await fetch("/api/remapping/validate", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    candidate_path: args.candidatePath || args.candidate_path,
                }),
            });
            return await res.json();
        }
        case "apply_remapping": {
            const res = await fetch("/api/remapping/apply", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    location_id: args.locationId || args.location_id,
                    new_path: args.newPath || args.new_path,
                }),
            });
            const data = await res.json();
            if (!res.ok || data.success === false) {
                throw new Error(data.error || "Falha ao aplicar remapeamento");
            }
            return data;
        }
        case "reset_remapping": {
            const res = await fetch("/api/remapping/reset", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    location_id: args.locationId || args.location_id,
                }),
            });
            const data = await res.json();
            if (!res.ok || data.success === false) {
                throw new Error(data.error || "Falha ao resetar remapeamento");
            }
            return data;
        }
        case "hide_window":
        case "minimize_window":
        case "toggle_maximize_window": {
            return;
        }
        default:
            throw new Error(`Comando não implementado: ${command}`);
    }
}

// ── Utilitários ───────────────────────────────────────────────────────────────

function escapeHtml(str) {
    if (str === null || str === undefined) return "";
    return String(str)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function isSafeHttpsUrl(url) {
    if (!url || typeof url !== "string") return false;
    if (url.length === 0 || url.length > 256) return false;
    // Rejeita qualquer espaço em branco ou caractere de controle
    if (/\s|[\x00-\x1F\x7F]/.test(url)) return false;
    if (!url.toLowerCase().startsWith("https://")) return false;
    const allowed = [
        "https://shared.cloudflare.steamstatic.com/",
        "https://cdn.cloudflare.steamstatic.com/",
        "https://steamcdn-a.akamaihd.net/",
    ];
    const match = allowed.find(domain => url.slice(0, domain.length).toLowerCase() === domain);
    if (!match) return false;
    const path = url.slice(match.length);
    if (
        !path ||
        path.includes("@") ||
        path.includes("?") ||
        path.includes("#") ||
        path.includes(":") ||
        path.includes("..") ||
        path.includes("//")
    ) {
        return false;
    }
    if (!/^[a-zA-Z0-9/_.-]+$/.test(path)) {
        return false;
    }
    const lower = path.toLowerCase();
    return lower.endsWith(".jpg") || lower.endsWith(".jpeg") || lower.endsWith(".png");
}

function getGamepadSvg(sizeClass = "w-4 h-4") {
    return `<svg class="${sizeClass} opacity-80 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="6" width="20" height="12" rx="4"></rect><line x1="6" y1="12" x2="10" y2="12"></line><line x1="8" y1="10" x2="8" y2="14"></line><line x1="15" y1="11" x2="15.01" y2="11"></line><line x1="18" y1="13" x2="18.01" y2="13"></line></svg>`;
}

function getGameInitials(name) {
    if (!name || typeof name !== "string") return "?";
    const trimmed = name.trim();
    if (!trimmed) return "?";
    const chars = Array.from(trimmed);
    const words = trimmed.split(/\s+/).filter(Boolean);
    if (words.length >= 2) {
        const first = Array.from(words[0])[0] || "";
        const second = Array.from(words[1])[0] || "";
        return (first + second).toUpperCase();
    }
    return chars.slice(0, 2).join("").toUpperCase();
}

function handleCoverError(img) {
    if (!img || !img.parentElement) return;
    const parent = img.parentElement;
    const gameName = img.getAttribute("data-game-name") || "?";
    const grad = img.getAttribute("data-grad") || "from-slate-700 to-slate-900";
    const size = img.getAttribute("data-size") || "md";
    const initials = getGameInitials(gameName);
    let svgSize = "w-4 h-4";
    if (size === "sm") svgSize = "w-3 h-3";
    else if (size === "lg") svgSize = "w-5 h-5";

    img.remove();
    parent.className = `${parent.className.replace(/bg-slate-\d+(\/\d+)?/g, "")} bg-gradient-to-br ${grad} text-white font-bold text-xs gap-1.5 px-1`;
    parent.innerHTML = `<span>${escapeHtml(initials)}</span>${getGamepadSvg(svgSize)}`;
}

function renderCoverHtml(coverUrl, gameName, grad, size = "md") {
    const initials = getGameInitials(gameName);
    const hasCover = coverUrl && isSafeHttpsUrl(coverUrl);

    let containerClass = "w-24 h-12 sm:w-28 sm:h-14 rounded-xl overflow-hidden shrink-0 shadow-xs relative border flex items-center justify-center";
    let svgSize = "w-4 h-4";

    if (size === "sm") {
        containerClass = "w-14 h-8 rounded-lg overflow-hidden shrink-0 shadow-xs relative border flex items-center justify-center";
        svgSize = "w-3 h-3";
    } else if (size === "lg") {
        containerClass = "w-24 h-12 sm:w-28 sm:h-14 rounded-xl overflow-hidden shrink-0 shadow-xs relative border flex items-center justify-center";
        svgSize = "w-4 h-4";
    }

    if (hasCover) {
        return `<div class="${containerClass} bg-slate-100 dark:bg-slate-800/80 border-slate-200/80 dark:border-slate-700/80">
            <img
                src="${escapeHtml(coverUrl)}"
                alt="Capa de ${escapeHtml(gameName || 'Jogo')}"
                data-game-name="${escapeHtml(gameName || '?')}"
                data-grad="${escapeHtml(grad || 'from-slate-700 to-slate-900')}"
                data-size="${escapeHtml(size || 'md')}"
                loading="lazy"
                class="w-full h-full object-cover transition-opacity duration-200"
                onerror="handleCoverError(this)"
            />
        </div>`;
    }

    return `<div class="${containerClass} bg-gradient-to-br ${grad} border-slate-200/40 dark:border-slate-700/40 text-white font-bold text-xs gap-1.5 px-1">
        <span>${escapeHtml(initials)}</span>
        ${getGamepadSvg(svgSize)}
    </div>`;
}

// ── Toasts & Notificações Modernas ───────────────────────────────────────────

function showToast(message, type = "success") {
    const container = document.getElementById("toast-container");
    if (!container) return;

    const toast = document.createElement("div");
    toast.className = "pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-2xl shadow-xl border text-xs font-semibold transform transition-all duration-300 translate-y-3 opacity-0";

    const icons = {
        success: "✅",
        error: "❌",
        warning: "⚠️",
        info: "ℹ️",
    };

    if (type === "success") {
        toast.classList.add("bg-white", "dark:bg-slate-900", "border-emerald-300", "dark:border-emerald-700", "text-slate-800", "dark:text-slate-100");
    } else if (type === "error") {
        toast.classList.add("bg-white", "dark:bg-slate-900", "border-red-300", "dark:border-red-700", "text-red-700", "dark:text-red-400");
    } else if (type === "warning") {
        toast.classList.add("bg-white", "dark:bg-slate-900", "border-amber-300", "dark:border-amber-700", "text-amber-800", "dark:text-amber-300");
    } else {
        toast.classList.add("bg-white", "dark:bg-slate-900", "border-slate-300", "dark:border-slate-700", "text-slate-800", "dark:text-slate-100");
    }

    toast.innerHTML = `
        <span class="text-base flex-shrink-0">${icons[type] || "ℹ️"}</span>
        <span class="flex-1 leading-snug">${escapeHtml(message)}</span>
        <button class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 ml-2 cursor-pointer" onclick="this.parentElement.remove()">✕</button>
    `;

    container.appendChild(toast);

    requestAnimationFrame(() => {
        toast.classList.remove("translate-y-3", "opacity-0");
        toast.classList.add("translate-y-0", "opacity-100");
    });

    setTimeout(() => {
        toast.classList.add("opacity-0", "translate-y-2");
        setTimeout(() => toast.remove(), 300);
    }, 3500);
}

// ── Modal de Confirmação Moderno ──────────────────────────────────────────────

let pendingConfirmAction = null;

function showConfirmDialog({ title, message, confirmText = "Confirmar", cancelText = "Cancelar", isDanger = false, onConfirm }) {
    const modal = document.getElementById("custom-confirm-modal");
    if (!modal) {
        if (confirm(`${title}\n\n${message}`)) {
            if (onConfirm) onConfirm();
        }
        return;
    }

    const titleEl = document.getElementById("confirm-modal-title");
    const msgEl = document.getElementById("confirm-modal-message");
    if (titleEl) titleEl.textContent = title;
    if (msgEl) msgEl.textContent = message;

    const okBtn = document.getElementById("confirm-modal-ok");
    const cancelBtn = document.getElementById("confirm-modal-cancel");
    if (okBtn) okBtn.textContent = confirmText;
    if (cancelBtn) cancelBtn.textContent = cancelText;

    if (okBtn) {
        if (isDanger) {
            okBtn.className = "px-4 py-2 bg-red-600 hover:bg-red-700 text-white text-xs font-semibold rounded-xl shadow-xs transition-colors cursor-pointer";
        } else {
            okBtn.className = "px-4 py-2 bg-slate-900 hover:bg-slate-800 dark:bg-emerald-600 dark:hover:bg-emerald-500 text-white text-xs font-semibold rounded-xl shadow-xs transition-colors cursor-pointer";
        }
    }

    pendingConfirmAction = onConfirm;
    modal.classList.remove("hidden");
}

function closeConfirmModal(confirmed) {
    const modal = document.getElementById("custom-confirm-modal");
    if (modal) modal.classList.add("hidden");
    if (confirmed && typeof pendingConfirmAction === "function") {
        pendingConfirmAction();
    }
    pendingConfirmAction = null;
}

// ── Tema Escuro (Dark Mode) ───────────────────────────────────────────────────

function toggleTheme() {
    const isDark = document.documentElement.classList.toggle("dark");
    localStorage.setItem("savevault-theme", isDark ? "dark" : "light");
    updateThemeUI(isDark);
    showToast(isDark ? "Modo Escuro ativado" : "Modo Claro ativado", "info");
}

function updateThemeUI(isDark) {
    const icon = document.getElementById("theme-toggle-icon");
    const label = document.getElementById("theme-toggle-label");
    if (icon) icon.textContent = isDark ? "☀️" : "🌙";
    if (label) label.textContent = isDark ? "Modo Claro" : "Modo Escuro";
}

function initTheme() {
    const isDark = document.documentElement.classList.contains("dark");
    updateThemeUI(isDark);
}

// ── Central de Ajuda & Guia Rápido ────────────────────────────────────────────

function openHelpModal(e) {
    if (e) e.preventDefault();
    document.getElementById("help-modal")?.classList.remove("hidden");
}

function closeHelpModal() {
    document.getElementById("help-modal")?.classList.add("hidden");
}

// ── Abrir Pasta no Windows Explorer ───────────────────────────────────────────

async function openFolder(path) {
    if (!path) return;
    try {
        await invoke("open_folder", { path });
        showToast("Pasta aberta no Windows Explorer", "info");
    } catch (e) {
        showToast(`Erro ao abrir pasta: ${e}`, "error");
    }
}

function timeAgo(isoString) {
    if (!isoString) return "—";
    const now = new Date();
    const then = new Date(isoString);
    const diff = Math.floor((now - then) / 1000);
    if (diff < 60) return "agora mesmo";
    if (diff < 3600) return `há ${Math.floor(diff / 60)} min`;
    if (diff < 86400) return `há ${Math.floor(diff / 3600)}h`;
    return then.toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
}

function formatHour(isoString) {
    if (!isoString) return "—";
    const d = new Date(isoString);
    const today = new Date();
    const prefix = d.toDateString() === today.toDateString() ? "Hoje" : d.toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
    return `${prefix}, ${d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}`;
}

function platformBadge(platform) {
    const icons = {
        "Steam": "🎮",
        "GOG": "🟣",
        "Dolphin": "🐬",
        "PCSX2": "💿",
        "Epic": "⚡",
    };
    return icons[platform] || "🎲";
}

// Cores de gradiente para avatares dos jogos (cicla baseado no índice)
const gradients = [
    "from-emerald-400 to-teal-600",
    "from-blue-400 to-indigo-600",
    "from-purple-400 to-pink-600",
    "from-orange-400 to-red-600",
    "from-yellow-400 to-orange-500",
    "from-cyan-400 to-blue-500",
];

// ── Controles da janela ───────────────────────────────────────────────────────

document.getElementById("titlebar-minimize")?.addEventListener("click", () => {
    if (hasTauri) {
        invoke("minimize_window").catch(() => appWindow.minimize());
    } else {
        appWindow.minimize();
    }
});

document.getElementById("titlebar-maximize")?.addEventListener("click", () => {
    if (hasTauri) {
        invoke("toggle_maximize_window").catch(() => appWindow.toggleMaximize());
    } else {
        appWindow.toggleMaximize();
    }
});

document.getElementById("titlebar-close")?.addEventListener("click", async () => {
    if (hasTauri) {
        try {
            await invoke("hide_window");
            return;
        } catch (_) {}
    }
    showToast("checkpoint continua rodando em segundo plano", "info");
    appWindow.close();
});

// ── Navegação de abas ─────────────────────────────────────────────────────────

const tabs = {
    "nav-inicio":       "tab-inicio",
    "nav-jogos":        "tab-jogos",
    "nav-atividade":    "tab-atividade",
    "nav-nuvem":        "tab-nuvem",
    "nav-config":       "tab-config",
};

function showTab(navId) {
    // Esconde todas as abas
    Object.values(tabs).forEach(tabId => {
        const el = document.getElementById(tabId);
        if (el) el.classList.add("hidden");
    });
    // Remove estado ativo de todos os links
    Object.keys(tabs).forEach(id => {
        const el = document.getElementById(id);
        if (el) el.classList.remove("bg-slate-100", "text-slate-900", "font-semibold");
    });
    // Mostra a aba selecionada
    const tabId = tabs[navId];
    const tab = document.getElementById(tabId);
    if (tab) tab.classList.remove("hidden");
    // Ativa o link
    const nav = document.getElementById(navId);
    if (nav) nav.classList.add("bg-slate-100", "text-slate-900", "font-semibold");
}

// Liga os cliques da sidebar
Object.keys(tabs).forEach(navId => {
    document.getElementById(navId)?.addEventListener("click", (e) => {
        e.preventDefault();
        showTab(navId);
        // Carrega dados da aba ao trocar
        if (navId === "nav-jogos") loadJogos();
        if (navId === "nav-atividade") loadAtividade();
        if (navId === "nav-nuvem") loadNuvem();
        if (navId === "nav-config") loadConfig();
    });
});

// Link "Ver todos" na aba Início
document.getElementById("link-ver-todos")?.addEventListener("click", (e) => {
    e.preventDefault();
    showTab("nav-jogos");
    loadJogos();
});

// ── Aba: Início ───────────────────────────────────────────────────────────────

async function loadInicio() {
    try {
        const data = await invoke("get_dashboard_status");

        // KPIs
        const elGames = document.getElementById("kpi-games");
        const elSaves = document.getElementById("kpi-saves");
        const elTime = document.getElementById("kpi-time");
        if (elGames) elGames.textContent = data.save_locations_count ?? 0;
        if (elSaves) elSaves.textContent = data.snapshots_count ?? 0;
        if (elTime) elTime.textContent = formatHour(data.last_reconciliation_time);

        // Status banner
        const banner = document.getElementById("status-banner");
        const bannerText = document.getElementById("status-text");
        if (data.sync_pending_count > 0) {
            banner?.classList.replace("bg-emerald-50", "bg-yellow-50");
            if (bannerText) bannerText.textContent = `${data.sync_pending_count} item(s) aguardando sincronização...`;
        } else {
            banner?.classList.replace("bg-yellow-50", "bg-emerald-50");
            if (bannerText) bannerText.textContent = "Tudo certo. Seus saves estão protegidos e sincronizados.";
        }
    } catch (e) {
        console.error("Erro ao carregar dashboard:", e);
    }

    // Jogos recentes (os primeiros da lista de locais)
    try {
        const locs = await invoke("get_locations");
        renderJogosRecentes(locs.slice(0, 4));
    } catch (e) {
        console.error("Erro ao carregar jogos recentes:", e);
    }

    // AUD-03: Verifica se há locais necessitando de remapeamento
    await checkPendingRemappings();
}

// ── Remapeamento de Saves Multiplataforma (AUD-03) ──────────────────────────

async function checkPendingRemappings() {
    try {
        const pending = await invoke("get_pending_remappings");
        const banner = document.getElementById("remapping-alert-banner");
        const text = document.getElementById("remapping-alert-text");
        if (pending && pending.length > 0) {
            if (banner) banner.classList.remove("hidden");
            if (text) {
                text.textContent = `${pending.length} local(is) de save recuperado(s) precisam de definição de pasta nesta máquina.`;
            }
        } else {
            if (banner) banner.classList.add("hidden");
        }
    } catch (e) {
        console.error("Erro ao verificar remapeamentos pendentes:", e);
    }
}

async function openRemappingModal() {
    const modal = document.getElementById("remapping-modal");
    const list = document.getElementById("remapping-items-list");
    if (!modal || !list) return;

    list.innerHTML = `<p class="text-xs text-slate-400">Carregando locais pendentes...</p>`;
    modal.classList.remove("hidden");

    try {
        const pending = await invoke("get_pending_remappings");
        if (!pending || pending.length === 0) {
            list.innerHTML = `
                <div class="text-center py-8">
                    <span class="text-3xl">✓</span>
                    <p class="text-xs font-semibold text-emerald-600 dark:text-emerald-400 mt-2">Nenhum remapeamento pendente!</p>
                    <p class="text-[11px] text-slate-400 mt-1">Todos os locais de save estão sincronizados e configurados corretamente.</p>
                </div>
            `;
            const banner = document.getElementById("remapping-alert-banner");
            if (banner) banner.classList.add("hidden");
            return;
        }

        list.innerHTML = pending.map(item => {
            const initialCandidate = item.suggested_candidate || "";
            return `
                <div class="bg-slate-50 dark:bg-slate-800/80 border border-slate-200 dark:border-slate-700/80 rounded-xl p-4 space-y-3" id="remap-card-${item.location_id}">
                    <div class="flex items-center justify-between">
                        <div class="flex items-center gap-2">
                            <span class="text-base">🎮</span>
                            <span class="text-xs font-bold text-slate-800 dark:text-slate-100">${escapeHtml(item.game_name)}</span>
                            <span class="text-[10px] bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300 px-2 py-0.5 rounded-full">${escapeHtml(item.location_type)}</span>
                        </div>
                        <span class="text-[10px] font-semibold px-2 py-0.5 rounded-full ${item.status === 'NeedsUserMapping' ? 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300' : 'bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-300'}">
                            ${escapeHtml(item.status)}
                        </span>
                    </div>

                    <div class="space-y-1 text-xs font-mono">
                        <div>
                            <span class="text-slate-400 font-sans text-[11px]">Local original:</span>
                            <p class="text-slate-700 dark:text-slate-300 bg-white dark:bg-slate-900 px-2.5 py-1.5 rounded-lg border border-slate-200/80 dark:border-slate-800 break-all select-all">
                                ${escapeHtml(item.original_path)}
                            </p>
                        </div>
                        <p class="text-[11px] text-amber-600 dark:text-amber-400 font-sans mt-0.5">
                            ⚠️ ${escapeHtml(item.reason)}
                        </p>
                    </div>

                    <div class="space-y-1.5">
                        <label class="block text-[11px] font-bold text-slate-700 dark:text-slate-300">Novo destino nesta máquina:</label>
                        <div class="flex gap-2">
                            <input
                                type="text"
                                id="remap-input-${item.location_id}"
                                value="${escapeHtml(initialCandidate)}"
                                placeholder="Ex: C:\\Users\\... ou /home/..."
                                class="flex-1 px-3 py-1.5 text-xs font-mono bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-xl focus:outline-none focus:ring-2 focus:ring-amber-500/20 text-slate-800 dark:text-slate-100 placeholder-slate-400"
                            />
                            <button
                                onclick="validateRemappingCandidate('${item.location_id}')"
                                class="px-3 py-1.5 bg-slate-200 hover:bg-slate-300 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 text-xs font-semibold rounded-xl transition-colors shrink-0 cursor-pointer"
                            >
                                Validar
                            </button>
                            <button
                                onclick="submitRemappingCandidate('${item.location_id}')"
                                class="px-3 py-1.5 bg-amber-600 hover:bg-amber-700 text-white text-xs font-semibold rounded-xl transition-colors shrink-0 cursor-pointer"
                            >
                                Salvar Mapeamento
                            </button>
                        </div>
                        <div id="remap-feedback-${item.location_id}" class="text-[11px] font-medium hidden"></div>
                    </div>
                </div>
            `;
        }).join("");
    } catch (e) {
        list.innerHTML = `<p class="text-xs text-red-500">Erro ao listar pendências: ${escapeHtml(e.message || e)}</p>`;
    }
}

function closeRemappingModal() {
    const modal = document.getElementById("remapping-modal");
    if (modal) modal.classList.add("hidden");
    checkPendingRemappings();
    loadInicio();
}

async function validateRemappingCandidate(locId) {
    const input = document.getElementById(`remap-input-${locId}`);
    const feedback = document.getElementById(`remap-feedback-${locId}`);
    if (!input || !feedback) return;

    const candidate = input.value.trim();
    if (!candidate) {
        feedback.textContent = "Por favor, digite um caminho de destino.";
        feedback.className = "text-[11px] font-medium text-red-500";
        feedback.classList.remove("hidden");
        return;
    }

    try {
        const res = await invoke("validate_remapping_path", { candidatePath: candidate });
        if (!res.is_valid) {
            feedback.textContent = `❌ ${res.error_message || "Caminho inválido para este sistema"}`;
            feedback.className = "text-[11px] font-medium text-red-500";
            feedback.classList.remove("hidden");
        } else {
            let msg = "✓ Caminho válido e seguro.";
            if (res.has_conflict) {
                msg += ` ⚠️ Atenção: pasta já existe e contém ${res.existing_files_count} arquivo(s).`;
                feedback.className = "text-[11px] font-medium text-amber-600 dark:text-amber-400";
            } else if (!res.exists_on_disk) {
                msg += " A pasta será criada automaticamente durante a restauração.";
                feedback.className = "text-[11px] font-medium text-blue-600 dark:text-blue-400";
            } else {
                feedback.className = "text-[11px] font-medium text-emerald-600 dark:text-emerald-400";
            }
            feedback.textContent = msg;
            feedback.classList.remove("hidden");
        }
    } catch (e) {
        feedback.textContent = `Erro ao validar: ${e.message || e}`;
        feedback.className = "text-[11px] font-medium text-red-500";
        feedback.classList.remove("hidden");
    }
}

async function submitRemappingCandidate(locId) {
    const input = document.getElementById(`remap-input-${locId}`);
    const feedback = document.getElementById(`remap-feedback-${locId}`);
    const card = document.getElementById(`remap-card-${locId}`);
    if (!input || !feedback) return;

    const newPath = input.value.trim();
    if (!newPath) {
        feedback.textContent = "Por favor, digite um caminho de destino válido.";
        feedback.className = "text-[11px] font-medium text-red-500";
        feedback.classList.remove("hidden");
        return;
    }

    try {
        await invoke("apply_remapping", { locationId: locId, newPath: newPath });
        feedback.textContent = "✓ Local remapeado! Os próximos restores deste jogo utilizarão este diretório.";
        feedback.className = "text-[11px] font-medium text-emerald-600 dark:text-emerald-400";
        feedback.classList.remove("hidden");
        showToast("Mapeamento salvo com sucesso!", "success");

        setTimeout(() => {
            if (card) card.remove();
            checkPendingRemappings();
        }, 1200);
    } catch (e) {
        feedback.textContent = `❌ Falha ao aplicar: ${e.message || e}`;
        feedback.className = "text-[11px] font-medium text-red-500";
        feedback.classList.remove("hidden");
    }
}

function renderJogosRecentes(locations) {
    const container = document.getElementById("jogos-recentes-list");
    if (!container) return;
    if (!locations || locations.length === 0) {
        container.innerHTML = `<p class="text-sm text-slate-400 px-1">Nenhum jogo detectado ainda.</p>`;
        return;
    }
    container.innerHTML = locations.map((loc, i) => {
        const grad = gradients[i % gradients.length];
        const coverEl = renderCoverHtml(loc.cover_url, loc.game_name, grad, "sm");

        return `
        <div class="flex items-center gap-3 py-2.5 px-1 border-b border-slate-100 dark:border-slate-800/80 last:border-0">
            ${coverEl}
            <div class="flex-1 min-w-0">
                <p class="text-sm font-semibold text-slate-800 dark:text-slate-100 truncate">${escapeHtml(loc.game_name || "Jogo")}</p>
                <p class="text-xs text-slate-400">${loc.files_count ?? 0} saves · ${timeAgo(loc.last_seen_at)}</p>
            </div>
            <span class="text-xs font-medium px-2 py-0.5 rounded-full bg-emerald-100 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-400 flex-shrink-0">Sincronizado ✓</span>
        </div>`;
    }).join("");
}

// ── Aba: Jogos ────────────────────────────────────────────────────────────────

let allJogosLocations = [];

async function loadJogos() {
    const container = document.getElementById("jogos-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando...</p>`;
    try {
        const locs = await invoke("get_locations");
        allJogosLocations = locs || [];
        const searchInput = document.getElementById("jogos-search-input");
        const currentQuery = searchInput ? searchInput.value.trim() : "";
        renderJogosList(currentQuery);
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
    }
}

function renderJogosList(query = "") {
    const container = document.getElementById("jogos-list");
    const statsEl = document.getElementById("jogos-search-stats");
    const clearBtn = document.getElementById("jogos-search-clear");
    if (!container) return;

    const trimmedQuery = (query || "").trim().toLowerCase();

    if (clearBtn) {
        if (trimmedQuery.length > 0) {
            clearBtn.classList.remove("hidden");
        } else {
            clearBtn.classList.add("hidden");
        }
    }

    if (!allJogosLocations || allJogosLocations.length === 0) {
        if (statsEl) statsEl.classList.add("hidden");
        container.innerHTML = `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">🎮</div>
                <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm">Nenhum jogo monitorado ainda</p>
                <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Clique em "<strong>+ Adicionar Jogo</strong>" para cadastrar seu save manualmente ou aguarde o escaneamento.</p>
            </div>
        `;
        return;
    }

    let filtered = allJogosLocations;
    if (trimmedQuery.length > 0) {
        filtered = allJogosLocations.filter(loc => {
            const name = (loc.game_name || "").toLowerCase();
            const path = (loc.path || "").toLowerCase();
            return name.includes(trimmedQuery) || path.includes(trimmedQuery);
        });

        if (statsEl) {
            statsEl.classList.remove("hidden");
            statsEl.textContent = `Mostrando ${filtered.length} de ${allJogosLocations.length} jogo(s) encontrado(s) para "${query}"`;
        }
    } else {
        if (statsEl) statsEl.classList.add("hidden");
    }

    if (filtered.length === 0) {
        container.innerHTML = `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">🔍</div>
                <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm">Nenhum jogo encontrado</p>
                <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Nenhum resultado corresponde à busca "<strong>${escapeHtml(query)}</strong>".</p>
                <button onclick="clearJogosSearch()" class="mt-3 px-3 py-1.5 bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 text-xs font-semibold rounded-lg transition-colors cursor-pointer">
                    Limpar pesquisa
                </button>
            </div>
        `;
        return;
    }

    container.innerHTML = filtered.map((loc, i) => {
        const grad = gradients[i % gradients.length];
        const safePath = (loc.path || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
        const coverHtml = renderCoverHtml(loc.cover_url, loc.game_name, grad, "md");

        return `
        <div onclick="openGameDetails('${loc.id}')" class="bg-white dark:bg-slate-800/90 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-3.5 sm:p-4 flex items-center justify-between gap-4 shadow-xs hover:border-slate-300 dark:hover:border-slate-600 hover:shadow-sm transition-all cursor-pointer group">
            <div class="flex items-center gap-3.5 min-w-0">
                ${coverHtml}
                <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                        <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm truncate group-hover:text-emerald-600 dark:group-hover:text-emerald-400 transition-colors">${escapeHtml(loc.game_name || "Jogo Desconhecido")}</p>
                        <span class="text-[10px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wider bg-slate-100 dark:bg-slate-700 text-slate-500 dark:text-slate-300 shrink-0">${escapeHtml(loc.platform || "PC")}</span>
                    </div>
                    <p class="text-xs text-slate-500 dark:text-slate-400 truncate mt-0.5 font-mono" title="${escapeHtml(loc.path || "")}">${escapeHtml(loc.path || "")}</p>
                    <div class="flex items-center gap-2 mt-1">
                        <span class="text-xs text-slate-400 dark:text-slate-500">${loc.files_count ?? 0} saves</span>
                        <span class="text-slate-300 dark:text-slate-600">•</span>
                        <span class="text-xs font-medium text-emerald-600 dark:text-emerald-400 flex items-center gap-1">
                            <span>Sincronizado</span>
                            <span class="text-[11px]">✓</span>
                        </span>
                    </div>
                </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
                <button
                    onclick="event.stopPropagation(); openFolder('${safePath}')"
                    class="px-2.5 py-1.5 bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 text-xs font-semibold rounded-xl transition-colors flex items-center gap-1 cursor-pointer"
                    title="Abrir pasta no Windows Explorer"
                >
                    <span>📂</span> Abrir
                </button>
            </div>
        </div>`;
    }).join("");
}

function clearJogosSearch() {
    const input = document.getElementById("jogos-search-input");
    if (input) {
        input.value = "";
        input.focus();
    }
    renderJogosList("");
}

// ── Modal: Adicionar Jogo Manualmente ──────────────────────────────────────────

function openAddGameModal() {
    const nameInput = document.getElementById("add-game-name");
    const pathInput = document.getElementById("add-game-path");
    if (nameInput) nameInput.value = "";
    if (pathInput) pathInput.value = "";
    document.getElementById("add-game-modal")?.classList.remove("hidden");
    if (nameInput) nameInput.focus();
}

function closeAddGameModal() {
    document.getElementById("add-game-modal")?.classList.add("hidden");
}

async function submitAddGame() {
    const nameInput = document.getElementById("add-game-name");
    const pathInput = document.getElementById("add-game-path");
    const name = nameInput ? nameInput.value.trim() : "";
    const path = pathInput ? pathInput.value.trim() : "";

    if (!name || !path) {
        showToast("Por favor, preencha o nome do jogo e o caminho da pasta de save.", "warning");
        return;
    }

    try {
        await invoke("add_game", { name, path });
        showToast(`Jogo "${name}" adicionado com sucesso!`, "success");
        closeAddGameModal();
        await loadJogos();
        await loadInicio();
    } catch (e) {
        showToast(`Erro ao adicionar jogo: ${e}`, "error");
    }
}

// ── Modal: Detalhes do Jogo ───────────────────────────────────────────────────

let currentGameDetailsLoc = null;

async function openGameDetails(locationId) {
    const loc = allJogosLocations.find(l => String(l.id) === String(locationId));
    if (!loc) return;
    currentGameDetailsLoc = loc;

    const modal = document.getElementById("game-details-modal");
    if (!modal) return;

    const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
    const avatar = document.getElementById("game-details-avatar");
    if (avatar) {
        avatar.outerHTML = renderCoverHtml(loc.cover_url, loc.game_name, "from-slate-700 to-slate-900", "lg").replace(
            'class="',
            'id="game-details-avatar" class="'
        );
    }

    const titleEl = document.getElementById("game-details-title");
    if (titleEl) titleEl.textContent = loc.game_name || "Jogo Desconhecido";

    const subEl = document.getElementById("game-details-subtitle");
    if (subEl) subEl.textContent = loc.path || "";

    const platEl = document.getElementById("game-details-platform");
    if (platEl) platEl.textContent = loc.platform || "PC / Manual";

    const countEl = document.getElementById("game-details-files-count");
    if (countEl) countEl.textContent = `${loc.files_count ?? 0} arquivo(s) monitorado(s)`;

    const openFolderBtn = document.getElementById("game-details-btn-open-folder");
    if (openFolderBtn) {
        openFolderBtn.onclick = () => openFolder(loc.path);
    }

    const scanBtn = document.getElementById("game-details-btn-scan");
    if (scanBtn) {
        scanBtn.onclick = async () => {
            try {
                scanBtn.textContent = "Escaneando...";
                await invoke("trigger_reconcile");
                showToast("Verificação concluída!", "success");
                await loadJogos();
                await loadGameDetailsSnapshots(loc.game_name);
            } catch (err) {
                showToast(`Erro ao escanear: ${err}`, "error");
            } finally {
                scanBtn.innerHTML = `<span>🔍</span> Fazer Scan Agora`;
            }
        };
    }

    await loadGameDetailsSnapshots(loc.game_name);
    modal.classList.remove("hidden");
}

function closeGameDetailsModal() {
    document.getElementById("game-details-modal")?.classList.add("hidden");
    currentGameDetailsLoc = null;
}

async function loadGameDetailsSnapshots(gameName) {
    const listEl = document.getElementById("game-details-snapshots-list");
    if (!listEl) return;
    listEl.innerHTML = `<p class="text-xs text-slate-400">Carregando snapshots...</p>`;

    try {
        if (!allSnapshots || allSnapshots.length === 0) {
            allSnapshots = await invoke("get_snapshots") || [];
        }
        const gameSnaps = allSnapshots.filter(s => (s.game_name || "").toLowerCase() === (gameName || "").toLowerCase());
        if (gameSnaps.length === 0) {
            listEl.innerHTML = `<p class="text-xs text-slate-400 py-2">Nenhum snapshot registrado ainda para este jogo.</p>`;
            return;
        }

        listEl.innerHTML = gameSnaps.slice(0, 6).map(s => {
            const versionLabel = `v${s.version || 1}`;
            const sizeLabel = s.size_formatted || (s.size_bytes ? formatBytes(s.size_bytes) : "—");
            const relTime = timeAgo(s.created_at);
            const safeFilePath = (s.file_path || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
            return `
                <div class="p-2.5 bg-slate-50 dark:bg-slate-800/80 border border-slate-200/70 dark:border-slate-700 rounded-xl flex items-center justify-between text-xs">
                    <div class="min-w-0 pr-2">
                        <div class="flex items-center gap-1.5">
                            <span class="font-bold text-slate-800 dark:text-slate-200">${escapeHtml(s.file_name || "save")}</span>
                            <span class="px-1.5 py-0.2 rounded bg-emerald-100 dark:bg-emerald-950 text-emerald-700 dark:text-emerald-400 font-semibold text-[10px]">${versionLabel}</span>
                        </div>
                        <p class="text-[11px] text-slate-400 dark:text-slate-500 mt-0.5">${sizeLabel} · ${relTime}</p>
                    </div>
                    <div class="flex items-center gap-1.5 shrink-0">
                        <button onclick="openFolder('${safeFilePath}')" class="px-2 py-1 bg-white dark:bg-slate-700 hover:bg-slate-100 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 rounded-lg border border-slate-200 dark:border-slate-600 text-[11px] font-medium transition-colors cursor-pointer" title="Abrir pasta no Explorer">
                            📂
                        </button>
                        <button onclick="restoreSnapshot('${s.id}', '${escapeHtml(s.game_name || "").replace(/'/g, "\\'")}', '${versionLabel}', '${escapeHtml(s.file_name || "").replace(/'/g, "\\'")}')" class="px-2.5 py-1 bg-slate-800 hover:bg-slate-700 dark:bg-emerald-600 dark:hover:bg-emerald-500 text-white rounded-lg text-[11px] font-semibold transition-colors cursor-pointer">
                            ↺ Restaurar
                        </button>
                    </div>
                </div>
            `;
        }).join("");
    } catch (e) {
        listEl.innerHTML = `<p class="text-xs text-red-500">Erro: ${e}</p>`;
    }
}

// ── Aba: Atividade ────────────────────────────────────────────────────────────

let allSnapshots = [];

async function loadAtividade() {
    const container = document.getElementById("atividade-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando histórico...</p>`;
    try {
        const snapshots = await invoke("get_snapshots");
        allSnapshots = snapshots || [];

        populateAtividadeGameFilter();

        const searchInput = document.getElementById("atividade-search-input");
        const currentQuery = searchInput ? searchInput.value.trim() : "";
        renderAtividadeList(currentQuery);
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${escapeHtml(e)}</p>`;
    }
}

function populateAtividadeGameFilter() {
    const select = document.getElementById("atividade-game-filter");
    if (!select) return;

    const currentSelected = select.value;
    const gameNames = [...new Set(allSnapshots.map(s => s.game_name).filter(Boolean))].sort();

    select.innerHTML = `<option value="">🎮 Todos os Jogos (${allSnapshots.length})</option>` +
        gameNames.map(name => {
            const count = allSnapshots.filter(s => s.game_name === name).length;
            const isSelected = name === currentSelected ? "selected" : "";
            return `<option value="${escapeHtml(name)}" ${isSelected}>${escapeHtml(name)} (${count})</option>`;
        }).join("");
}

function onAtividadeFilterChange() {
    const searchInput = document.getElementById("atividade-search-input");
    const query = searchInput ? searchInput.value.trim() : "";
    renderAtividadeList(query);
}

function renderAtividadeList(query = "") {
    const container = document.getElementById("atividade-list");
    const statsEl = document.getElementById("atividade-search-stats");
    const clearBtn = document.getElementById("atividade-search-clear");
    const gameFilterEl = document.getElementById("atividade-game-filter");
    const selectedGame = gameFilterEl ? gameFilterEl.value.trim() : "";
    if (!container) return;

    const trimmedQuery = (query || "").trim().toLowerCase();

    if (clearBtn) {
        if (trimmedQuery.length > 0) {
            clearBtn.classList.remove("hidden");
        } else {
            clearBtn.classList.add("hidden");
        }
    }

    if (!allSnapshots || allSnapshots.length === 0) {
        if (statsEl) statsEl.classList.add("hidden");
        container.innerHTML = `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">📦</div>
                <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm">Nenhum snapshot capturado ainda</p>
                <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Quando você jogar ou salvar o progresso, o SaveVault criará pontos de restauração aqui.</p>
            </div>
        `;
        return;
    }

    let filtered = allSnapshots;

    // Filtro por jogo selecionado no dropdown
    if (selectedGame) {
        filtered = filtered.filter(s => (s.game_name || "") === selectedGame);
    }

    // Filtro por texto da busca
    if (trimmedQuery.length > 0) {
        filtered = filtered.filter(s => {
            const game = (s.game_name || "").toLowerCase();
            const file = (s.file_name || "").toLowerCase();
            const path = (s.file_path || "").toLowerCase();
            const version = `v${s.version || ""}`.toLowerCase();
            const hash = (s.hash || "").toLowerCase();
            return game.includes(trimmedQuery)
                || file.includes(trimmedQuery)
                || path.includes(trimmedQuery)
                || version.includes(trimmedQuery)
                || hash.includes(trimmedQuery);
        });
    }

    if (trimmedQuery.length > 0 || selectedGame) {
        if (statsEl) {
            statsEl.classList.remove("hidden");
            const filterLabel = selectedGame ? ` em "${selectedGame}"` : "";
            const searchLabel = trimmedQuery ? ` com "${query}"` : "";
            statsEl.textContent = `Mostrando ${filtered.length} de ${allSnapshots.length} snapshot(s)${filterLabel}${searchLabel}`;
        }
    } else {
        if (statsEl) statsEl.classList.add("hidden");
    }

    if (filtered.length === 0) {
        container.innerHTML = `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">🔍</div>
                <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm">Nenhum snapshot encontrado</p>
                <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">Nenhum snapshot corresponde aos filtros aplicados.</p>
                <button onclick="clearAtividadeSearch()" class="mt-3 px-3 py-1.5 bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 text-xs font-semibold rounded-lg transition-colors cursor-pointer">
                    Limpar filtros
                </button>
            </div>
        `;
        return;
    }

    container.innerHTML = filtered.map(s => {
        const gameTitle = s.game_name || "Jogo Desconhecido";
        const fileName = s.file_name || "save";
        const versionLabel = `v${s.version || 1}`;
        const sizeLabel = s.size_formatted || (s.size_bytes ? formatBytes(s.size_bytes) : "—");
        const hashShort = s.short_hash || (s.hash ? s.hash.substring(0, 10) : "—");
        const formattedDate = formatHour(s.created_at);
        const relativeDate = timeAgo(s.created_at);
        const safeFilePath = (s.file_path || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");

        return `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-4 shadow-xs hover:border-slate-300 dark:hover:border-slate-600 transition-colors flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                <div class="flex items-start gap-3 min-w-0">
                    <div class="w-9 h-9 rounded-xl bg-emerald-50 dark:bg-emerald-950/60 border border-emerald-200 dark:border-emerald-800 flex items-center justify-center text-emerald-600 dark:text-emerald-400 font-bold text-xs shrink-0 mt-0.5">
                        ${versionLabel}
                    </div>
                    <div class="min-w-0 flex-1">
                        <div class="flex items-center gap-2 flex-wrap">
                            <span class="font-bold text-slate-900 dark:text-slate-100 text-sm">${escapeHtml(gameTitle)}</span>
                            <span class="px-2 py-0.5 rounded-full text-xs font-semibold bg-emerald-100 dark:bg-emerald-950/80 text-emerald-800 dark:text-emerald-300">
                                ${versionLabel}
                            </span>
                            <span class="text-xs font-mono font-medium text-slate-600 dark:text-slate-300 bg-slate-100 dark:bg-slate-700/60 px-2 py-0.5 rounded-md truncate max-w-xs" title="${escapeHtml(s.file_path || fileName)}">
                                📄 ${escapeHtml(fileName)}
                            </span>
                        </div>
                        <div class="text-xs text-slate-500 dark:text-slate-400 mt-1 flex items-center gap-2 flex-wrap">
                            <span>📦 ${escapeHtml(sizeLabel)}</span>
                            <span>•</span>
                            <span title="${escapeHtml(s.created_at || "")}">🕒 ${escapeHtml(relativeDate)} (${escapeHtml(formattedDate)})</span>
                            <span>•</span>
                            <span class="font-mono text-slate-400 dark:text-slate-500" title="Hash Blake3: ${escapeHtml(s.hash || "")}">#${escapeHtml(hashShort)}</span>
                        </div>
                    </div>
                </div>
                <div class="flex items-center gap-2 shrink-0 self-end sm:self-center">
                    <button
                        onclick="openFolder('${safeFilePath}')"
                        class="px-2.5 py-1.5 rounded-xl bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 text-xs font-semibold transition-colors flex items-center gap-1 cursor-pointer"
                        title="Abrir pasta no Windows Explorer"
                    >
                        <span>📂</span> Abrir
                    </button>
                    <button
                        onclick="restoreSnapshot('${s.id}', '${escapeHtml(gameTitle).replace(/'/g, "\\'")}', '${versionLabel}', '${escapeHtml(fileName).replace(/'/g, "\\'")}')"
                        class="px-3.5 py-1.5 rounded-xl bg-slate-800 hover:bg-slate-700 dark:bg-emerald-600 dark:hover:bg-emerald-500 text-white text-xs font-semibold shadow-xs transition-colors flex items-center gap-1.5 cursor-pointer">
                        <span>↺</span> Restaurar Save
                    </button>
                </div>
            </div>`;
    }).join("");
}

function clearAtividadeSearch() {
    const input = document.getElementById("atividade-search-input");
    if (input) input.value = "";
    const filterEl = document.getElementById("atividade-game-filter");
    if (filterEl) filterEl.value = "";
    renderAtividadeList("");
}

function formatBytes(bytes) {
    if (!bytes || bytes <= 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
}

function restoreSnapshot(id, gameName, version, fileName) {
    showConfirmDialog({
        title: "Restaurar Save",
        message: `Deseja restaurar o snapshot ${version} de "${gameName}" (${fileName})?\n\nUm Safety Backup do save atual será criado automaticamente antes de restaurar o arquivo.`,
        confirmText: "Restaurar Agora",
        cancelText: "Cancelar",
        isDanger: true,
        onConfirm: async () => {
            try {
                const res = await invoke("restore_snapshot", { snapshotId: id, snapshot_id: id });
                if (res && res.success === false) {
                    showToast(`Erro ao restaurar: ${res.error || "Falha na restauração"}`, "error");
                    return;
                }
                showToast(`Save de "${gameName}" restaurado com sucesso! Safety backup criado.`, "success");
                await loadAtividade();
                await loadInicio();
            } catch (e) {
                const errMsg = e && e.message ? e.message : String(e);
                showToast(`Erro ao restaurar snapshot: ${errMsg}`, "error");
            }
        }
    });
}

// ── Aba: Nuvem ────────────────────────────────────────────────────────────────

let currentCloudConfig = null;

async function loadNuvem() {
    const container = document.getElementById("nuvem-info");
    if (!container) return;
    try {
        const [dashData, cloudConfig] = await Promise.all([
            invoke("get_dashboard_status"),
            invoke("get_cloud_config")
        ]);
        currentCloudConfig = cloudConfig;

        const isEnabled = cloudConfig.enabled && cloudConfig.provider !== "none";
        const providerNames = {
            "onedrive": "Microsoft OneDrive",
            "gdrive": "Google Drive",
            "google_drive": "Google Drive",
            "dropbox": "Dropbox",
            "local_folder": "Pasta Local / Nuvem Personalizada",
            "none": "Nuvem Desativada / Pausada",
        };
        const providerIcons = {
            "onedrive": "☁️",
            "gdrive": "📁",
            "google_drive": "📁",
            "dropbox": "📦",
            "local_folder": "💾",
            "none": "⏸️",
        };

        const providerKey = (cloudConfig.provider || "none").toLowerCase();
        const displayProvider = providerNames[providerKey] || (cloudConfig.provider || "Pasta Local");
        const displayIcon = providerIcons[providerKey] || "☁️";
        const syncPath = cloudConfig.local_sync_path || "Nenhuma pasta configurada";
        const safeSyncPath = syncPath.replace(/\\/g, "\\\\").replace(/'/g, "\\'");

        container.innerHTML = `
            <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-5 shadow-xs space-y-4">
                <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                    <div class="flex items-center gap-3">
                        <div class="w-11 h-11 rounded-xl ${isEnabled ? 'bg-blue-50 dark:bg-blue-950/60 text-blue-500 dark:text-blue-400' : 'bg-slate-100 dark:bg-slate-700 text-slate-400'} flex items-center justify-center text-2xl shrink-0">
                            ${displayIcon}
                        </div>
                        <div>
                            <div class="flex items-center gap-2">
                                <p class="font-bold text-slate-800 dark:text-slate-100 text-base">${escapeHtml(displayProvider)}</p>
                                <span class="text-xs font-semibold px-2 py-0.5 rounded-full ${isEnabled ? 'bg-emerald-100 dark:bg-emerald-950 text-emerald-700 dark:text-emerald-300' : 'bg-amber-100 dark:bg-amber-950 text-amber-700 dark:text-amber-300'}">
                                    ${isEnabled ? "Sincronização Ativa" : "Pausada"}
                                </span>
                            </div>
                            <p class="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                                ${isEnabled ? "Os backups dos seus jogos são espelhados para o provedor de nuvem." : "A sincronização para nuvem está temporariamente em pausa."}
                            </p>
                        </div>
                    </div>

                    <div class="flex items-center gap-2 shrink-0">
                        <button
                            onclick="toggleCloudSync()"
                            class="px-3 py-2 rounded-xl text-xs font-semibold transition-colors cursor-pointer ${isEnabled ? 'bg-amber-50 hover:bg-amber-100 dark:bg-amber-950/40 text-amber-700 dark:text-amber-300 border border-amber-200 dark:border-amber-800' : 'bg-emerald-600 hover:bg-emerald-500 text-white'}"
                        >
                            ${isEnabled ? "⏸️ Pausar Sincronização" : "▶️ Ativar Sincronização"}
                        </button>
                    </div>
                </div>

                <div class="bg-slate-50 dark:bg-slate-900/50 rounded-xl p-3.5 border border-slate-200/80 dark:border-slate-700/60 flex flex-col sm:flex-row sm:items-center justify-between gap-3 text-xs">
                    <div class="min-w-0 flex-1">
                        <span class="text-slate-400 font-semibold block text-[11px] uppercase tracking-wider mb-0.5">Pasta Sincronizada</span>
                        <p class="font-mono text-slate-800 dark:text-slate-200 truncate" title="${escapeHtml(syncPath)}">${escapeHtml(syncPath)}</p>
                    </div>
                    ${cloudConfig.local_sync_path ? `
                        <button
                            onclick="openFolder('${safeSyncPath}')"
                            class="px-3 py-1.5 bg-white dark:bg-slate-800 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 border border-slate-200 dark:border-slate-700 rounded-lg text-xs font-semibold transition-colors shrink-0 flex items-center gap-1.5 cursor-pointer shadow-2xs"
                        >
                            <span>📂</span> Abrir no Windows Explorer
                        </button>
                    ` : ""}
                </div>

                <div class="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1 text-xs text-slate-600 dark:text-slate-300">
                    <div class="bg-slate-50 dark:bg-slate-900/50 p-3 rounded-xl border border-slate-200/80 dark:border-slate-700/60">
                        <p class="text-slate-400 text-[11px]">Itens na fila de sync</p>
                        <p class="font-bold text-slate-800 dark:text-slate-100 text-sm mt-0.5">${dashData.sync_pending_count ?? 0} pendente(s)</p>
                    </div>
                    <div class="bg-slate-50 dark:bg-slate-900/50 p-3 rounded-xl border border-slate-200/80 dark:border-slate-700/60">
                        <p class="text-slate-400 text-[11px]">Última verificação / sync</p>
                        <p class="font-bold text-slate-800 dark:text-slate-100 text-sm mt-0.5">${formatHour(dashData.last_reconciliation_time)}</p>
                    </div>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro ao carregar status da nuvem: ${e}</p>`;
    }
}

async function toggleCloudSync() {
    if (!currentCloudConfig) return;
    const newEnabled = !currentCloudConfig.enabled || currentCloudConfig.provider === "none";
    const newProvider = newEnabled ? (currentCloudConfig.provider === "none" ? "local_folder" : currentCloudConfig.provider) : "none";

    try {
        await invoke("update_cloud_config", {
            provider: newProvider,
            localSyncPath: currentCloudConfig.local_sync_path,
            enabled: newEnabled,
        });
        showToast(newEnabled ? "Sincronização em nuvem retomada!" : "Sincronização em nuvem pausada.", "info");
        await loadNuvem();
        await loadInicio();
    } catch (e) {
        showToast(`Erro ao alterar estado da nuvem: ${e}`, "error");
    }
}

// ── Modal: Configurar Sincronização em Nuvem ──────────────────────────────────

let wizardDetectedCloudFolders = [];

async function openCloudConfigModal() {
    const modal = document.getElementById("cloud-config-modal");
    if (!modal) return;

    const listEl = document.getElementById("cloud-modal-detected-list");
    const customInput = document.getElementById("cloud-modal-custom-path");

    if (listEl) {
        listEl.innerHTML = `<p class="text-xs text-slate-400">Detectando pastas de nuvem...</p>`;
    }

    modal.classList.remove("hidden");

    try {
        const [detected, currentConfig] = await Promise.all([
            invoke("get_cloud_folders"),
            invoke("get_cloud_config")
        ]);

        wizardDetectedCloudFolders = detected || [];
        currentCloudConfig = currentConfig;

        if (customInput) {
            customInput.value = currentConfig?.local_sync_path || "";
        }

        renderCloudModalDetectedList(currentConfig?.local_sync_path);
    } catch (e) {
        if (listEl) {
            listEl.innerHTML = `<p class="text-xs text-red-500">Erro ao carregar pastas: ${e}</p>`;
        }
    }
}

function closeCloudConfigModal() {
    document.getElementById("cloud-config-modal")?.classList.add("hidden");
}

function renderCloudModalDetectedList(currentPath = "") {
    const listEl = document.getElementById("cloud-modal-detected-list");
    if (!listEl) return;

    if (!wizardDetectedCloudFolders || wizardDetectedCloudFolders.length === 0) {
        listEl.innerHTML = `
            <div class="p-3 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-800 rounded-xl text-amber-800 dark:text-amber-300 text-xs">
                Nenhum serviço de nuvem comum (Google Drive, OneDrive, Dropbox) foi detectado automaticamente nas pastas padrão. Você pode definir uma pasta personalizada abaixo.
            </div>
        `;
        return;
    }

    listEl.innerHTML = wizardDetectedCloudFolders.map(df => {
        const safeFolder = (df.savevault_folder || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
        const isCurrent = currentPath && df.savevault_folder && currentPath.toLowerCase() === df.savevault_folder.toLowerCase();
        const icon = (df.name || "").toLowerCase().includes("drive") ? "📁" : "☁️";

        return `
            <div class="flex items-center justify-between p-3 bg-slate-50 dark:bg-slate-800/80 border ${isCurrent ? 'border-emerald-500 ring-1 ring-emerald-500' : 'border-slate-200 dark:border-slate-700'} rounded-xl text-xs">
                <div class="flex items-center gap-2.5 min-w-0 mr-2">
                    <span class="text-xl">${icon}</span>
                    <div class="truncate">
                        <div class="flex items-center gap-1.5">
                            <span class="font-bold text-slate-800 dark:text-slate-100">${escapeHtml(df.name)}</span>
                            ${isCurrent ? '<span class="text-[10px] font-bold px-1.5 py-0.5 rounded bg-emerald-100 dark:bg-emerald-950 text-emerald-700 dark:text-emerald-400">Ativo</span>' : ''}
                        </div>
                        <p class="text-[11px] text-slate-500 dark:text-slate-400 font-mono truncate mt-0.5">${escapeHtml(df.savevault_folder)}</p>
                    </div>
                </div>
                <button
                    onclick="selectCloudModalFolder('${safeFolder}', '${escapeHtml(df.name).replace(/'/g, "\\'")}')"
                    class="px-3 py-1.5 bg-white dark:bg-slate-700 hover:bg-slate-100 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 font-semibold border border-slate-300 dark:border-slate-600 rounded-lg transition-colors shrink-0 cursor-pointer text-xs"
                >
                    Selecionar
                </button>
            </div>
        `;
    }).join("");
}

function selectCloudModalFolder(path, name) {
    const customInput = document.getElementById("cloud-modal-custom-path");
    if (customInput) {
        customInput.value = path;
    }
    showToast(`Pasta de ${name} selecionada`, "info");
}

async function submitCloudConfig() {
    const customInput = document.getElementById("cloud-modal-custom-path");
    const path = customInput ? customInput.value.trim() : "";

    if (!path) {
        showToast("Por favor, selecione ou informe o caminho da pasta de sincronização.", "warning");
        return;
    }

    let provider = "local_folder";
    const pathLower = path.toLowerCase();
    if (pathLower.includes("onedrive")) {
        provider = "onedrive";
    } else if (pathLower.includes("drive") || pathLower.includes("google")) {
        provider = "gdrive";
    } else if (pathLower.includes("dropbox")) {
        provider = "dropbox";
    }

    try {
        await invoke("update_cloud_config", {
            provider,
            localSyncPath: path,
            enabled: true,
        });
        showToast("Configuração da nuvem salva com sucesso!", "success");
        closeCloudConfigModal();
        await loadNuvem();
        await loadInicio();
    } catch (e) {
        showToast(`Erro ao salvar nuvem: ${e}`, "error");
    }
}

// ── Aba: Configurações & Manutenção ───────────────────────────────────────────

async function loadConfig() {
    const container = document.getElementById("config-info");
    if (!container) return;
    try {
        const data = await invoke("get_dashboard_status");
        container.innerHTML = `
            <div class="space-y-3">
                <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 dark:text-slate-400 uppercase tracking-wider mb-1">Banco de Dados</p>
                    <p class="text-sm text-slate-700 dark:text-slate-200 font-mono break-all">${data.db_path || "—"}</p>
                </div>
                <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 dark:text-slate-400 uppercase tracking-wider mb-1">Pasta de Backups</p>
                    <p class="text-sm text-slate-700 dark:text-slate-200 font-mono break-all">${data.backup_root || "—"}</p>
                </div>
                <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 dark:text-slate-400 uppercase tracking-wider mb-1">Intervalo de Reconciliação</p>
                    <p class="text-sm text-slate-700 dark:text-slate-200">${data.reconciliation_interval_secs ?? 30}s</p>
                </div>
                <div class="bg-white dark:bg-slate-800 border border-slate-200/90 dark:border-slate-700/80 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 dark:text-slate-400 uppercase tracking-wider mb-1">Status do Watcher</p>
                    <p class="text-sm text-slate-700 dark:text-slate-200">${data.watcher_status || "—"}</p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
    }
}

async function triggerOptimizeDatabase() {
    const btn = document.getElementById("btn-optimize-db");
    if (btn) {
        btn.disabled = true;
        btn.innerHTML = `<span class="animate-spin inline-block mr-1">🔄</span> Otimizando...`;
    }

    try {
        const res = await invoke("optimize_database");
        showToast(`SQLite otimizado! ${res.purged_sync_items || 0} itens antigos limpos. Tamanho atual: ${res.db_size_formatted || "OK"}`, "success");
        await loadConfig();
    } catch (e) {
        showToast(`Erro ao otimizar banco: ${e}`, "error");
    } finally {
        if (btn) {
            btn.disabled = false;
            btn.innerHTML = `<span>⚡</span> Otimizar SQLite Agora`;
        }
    }
}

async function triggerExportZip() {
    const btn = document.getElementById("btn-export-zip");
    if (btn) {
        btn.disabled = true;
        btn.innerHTML = `<span class="animate-spin inline-block mr-1">🔄</span> Compactando...`;
    }

    try {
        const res = await invoke("export_backup_zip");
        showToast(`Backup ZIP gerado com sucesso (${res.file_size_formatted || ""}): ${res.export_path}`, "success");
        if (res.export_path) {
            openFolder(res.export_path);
        }
    } catch (e) {
        showToast(`Erro ao exportar backup: ${e}`, "error");
    } finally {
        if (btn) {
            btn.disabled = false;
            btn.innerHTML = `<span>💾</span> Exportar para .ZIP`;
        }
    }
}

// ── Botão Reconciliar ─────────────────────────────────────────────────────────

document.getElementById("btn-reconcile")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-reconcile");
    if (btn) {
        btn.textContent = "Sincronizando...";
        btn.disabled = true;
    }
    try {
        const result = await invoke("trigger_reconcile");
        showToast(`Sincronização concluída: ${result}`, "success");
        loadInicio();
    } catch (e) {
        showToast(`Erro na sincronização: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "Sincronizar Agora";
            btn.disabled = false;
        }
    }
});

// ── Onboarding & Setup Modernizado (Apple/macOS Style) ────────────────────────

let currentOnboardingStep = 1;
let onboardingAvailableLibraries = [];
let onboardingSelectedLibraries = new Set(["steam", "gog", "emulators", "generic"]);
let onboardingStorageMode = "automatic"; // "automatic" | "custom" | "onedrive" | etc.
let onboardingStoragePath = "";
let onboardingInterfaceMode = "basic"; // "basic" | "advanced" | "ultra_light"
let onboardingDetectedCloudFolders = [];

async function checkInitialSetup() {
    try {
        const status = await invoke("get_setup_status");
        if (status) {
            if (status.interface_mode) {
                applyInterfaceMode(status.interface_mode);
            }
            if (!status.setup_completed) {
                openSetupWizard();
            }
        }
    } catch (e) {
        console.warn("Não foi possível verificar status do setup inicial:", e);
    }
}

async function openSetupWizard() {
    currentOnboardingStep = 1;
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.remove("hidden");
    }
    dismissOnboardingError();
    onboardingGoToStep(1);
    await loadOnboardingData();
}

function closeSetupWizard() {
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.add("hidden");
    }
    dismissOnboardingError();
}

function dismissOnboardingError() {
    const banner = document.getElementById("onboarding-error-banner");
    if (banner) {
        banner.classList.add("hidden");
    }
}

function showOnboardingError(message) {
    const banner = document.getElementById("onboarding-error-banner");
    const msgEl = document.getElementById("onboarding-error-message");
    if (banner && msgEl) {
        msgEl.textContent = message;
        banner.classList.remove("hidden");
    }
}

const sleep = (ms) => new Promise(resolve => setTimeout(resolve, ms));

async function loadOnboardingData() {
    // 1. Inicia exibindo o estado "Procurando..." de forma imediata e assíncrona
    if (onboardingAvailableLibraries.length === 0) {
        onboardingAvailableLibraries = [
            { id: "steam", name: "Steam", category: "commercial", detecting: true, detected: false, description: "Jogos e saves instalados via Steam" },
            { id: "gog", name: "GOG Galaxy", category: "commercial", detecting: true, detected: false, description: "Jogos instalados pelo GOG Galaxy ou DRM-free" },
            { id: "emulators", name: "Emuladores", category: "emulators", detecting: true, detected: false, description: "Procure saves de emuladores instalados ou configurados posteriormente." },
            { id: "generic", name: "Jogos genéricos", category: "generic", detecting: false, detected: true, description: "Para jogos que não pertencem a uma biblioteca conhecida." },
        ];
        renderOnboardingLibraries();
    }

    // 2. Busca bibliotecas de forma assíncrona sem travar a interface
    invoke("get_available_libraries")
        .then(libs => {
            if (Array.isArray(libs) && libs.length > 0) {
                onboardingAvailableLibraries = libs.map(l => ({ ...l, detecting: false }));
            } else {
                onboardingAvailableLibraries = onboardingAvailableLibraries.map(l => ({ ...l, detecting: false }));
            }
            renderOnboardingLibraries();
        })
        .catch(e => {
            console.warn("Erro ao buscar bibliotecas para onboarding:", e);
            onboardingAvailableLibraries = onboardingAvailableLibraries.map(l => ({ ...l, detecting: false }));
            renderOnboardingLibraries();
        });

    // 3. Carrega pastas de sincronização de nuvem e autostart existente
    try {
        const status = await invoke("get_setup_status");
        if (status) {
            if (Array.isArray(status.detected_cloud_folders)) {
                onboardingDetectedCloudFolders = status.detected_cloud_folders;
                renderOnboardingCloudFolders();
            }
            if (typeof status.autostart_enabled === "boolean") {
                const toggle = document.getElementById("onboarding-toggle-autostart");
                if (toggle) {
                    toggle.checked = status.autostart_enabled;
                    onboardingAutostartToggleChanged();
                }
            }
        }
    } catch (e) {
        console.warn("Erro ao carregar status inicial de nuvem:", e);
    }
}

function renderOnboardingLibraries() {
    const storeContainer = document.getElementById("onboarding-libraries-store-container");
    const otherContainer = document.getElementById("onboarding-libraries-other-container");

    if (storeContainer) {
        const commercial = onboardingAvailableLibraries.filter(l => l.category === "commercial");
        if (commercial.length === 0) {
            storeContainer.innerHTML = `<p class="text-xs text-slate-400 py-2">Nenhuma biblioteca comercial encontrada.</p>`;
        } else {
            let html = "";
            commercial.forEach(lib => {
                const isSelected = onboardingSelectedLibraries.has(lib.id);
                const borderClass = isSelected
                    ? "border-emerald-500 bg-emerald-50/20 dark:bg-emerald-950/20"
                    : "border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40";
                const checkClass = isSelected
                    ? "border-emerald-500 bg-emerald-500 text-white font-bold text-xs"
                    : "border-slate-300 dark:border-slate-600";
                const checkContent = isSelected ? "✓" : "";

                let statusBadge = "";
                if (lib.detecting) {
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-medium text-amber-700 dark:text-amber-300 bg-amber-100/70 dark:bg-amber-900/40 px-2 py-0.5 rounded-full">
                         <span class="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse"></span> Procurando…
                       </span>`;
                } else if (lib.detected) {
                    const text = lib.id === "steam" ? "Biblioteca Steam encontrada" : "Biblioteca GOG encontrada";
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-semibold text-emerald-700 dark:text-emerald-400 bg-emerald-100/70 dark:bg-emerald-900/50 px-2 py-0.5 rounded-full">
                         <span class="w-1.5 h-1.5 rounded-full bg-emerald-500"></span> ${text}
                       </span>`;
                } else {
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-medium text-slate-400 dark:text-slate-500 bg-slate-100 dark:bg-slate-800 px-2 py-0.5 rounded-full">
                         Nenhuma instalação encontrada
                       </span>`;
                }

                const pathDisplay = (lib.detected && lib.path)
                    ? `<p class="text-[11px] text-slate-400 font-mono truncate mt-0.5">${escapeHtml(lib.path)}</p>`
                    : "";

                html += `
                    <div onclick="toggleOnboardingLibrary('${lib.id}')"
                         onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();toggleOnboardingLibrary('${lib.id}');}"
                         id="lib-card-${lib.id}"
                         tabindex="0"
                         role="checkbox"
                         aria-checked="${isSelected}"
                         class="onboarding-card p-4 rounded-2xl border-2 ${borderClass} cursor-pointer flex items-center justify-between gap-3 transition-all focus:outline-none">
                        <div class="flex items-center gap-3.5 min-w-0">
                            <div class="w-10 h-10 rounded-xl bg-slate-100 dark:bg-slate-800 flex items-center justify-center text-lg shrink-0">
                                🎮
                            </div>
                            <div class="min-w-0">
                                <div class="flex items-center gap-2">
                                    <h4 class="text-sm font-bold text-slate-900 dark:text-slate-100">${escapeHtml(lib.name)}</h4>
                                    ${statusBadge}
                                </div>
                                <p class="text-xs text-slate-500 dark:text-slate-400 mt-0.5">${escapeHtml(lib.description || "")}</p>
                                ${pathDisplay}
                            </div>
                        </div>
                        <div id="lib-check-${lib.id}" class="w-6 h-6 rounded-full border-2 ${checkClass} flex items-center justify-center shrink-0">
                            ${checkContent}
                        </div>
                    </div>
                `;
            });
            storeContainer.innerHTML = html;
        }
    }

    if (otherContainer) {
        const others = onboardingAvailableLibraries.filter(l => l.category !== "commercial");
        let html = "";
        others.forEach(lib => {
            const isSelected = onboardingSelectedLibraries.has(lib.id);
            const borderClass = isSelected
                ? "border-emerald-500 bg-emerald-50/20 dark:bg-emerald-950/20"
                : "border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40";
            const checkClass = isSelected
                ? "border-emerald-500 bg-emerald-500 text-white font-bold text-xs"
                : "border-slate-300 dark:border-slate-600";
            const checkContent = isSelected ? "✓" : "";
            const icon = lib.id === "emulators" ? "🕹️" : "📦";

            let statusBadge = "";
            let description = lib.description || "";
            if (lib.id === "emulators") {
                description = "Procure saves de emuladores instalados ou configurados posteriormente.";
                if (lib.detecting) {
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-medium text-amber-700 dark:text-amber-300 bg-amber-100/70 dark:bg-amber-900/40 px-2 py-0.5 rounded-full">
                         <span class="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse"></span> Procurando…
                       </span>`;
                } else if (lib.detected) {
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-semibold text-emerald-700 dark:text-emerald-400 bg-emerald-100/70 dark:bg-emerald-900/50 px-2 py-0.5 rounded-full">
                         <span class="w-1.5 h-1.5 rounded-full bg-emerald-500"></span> Emuladores detectados
                       </span>`;
                } else {
                    statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-medium text-slate-400 dark:text-slate-500 bg-slate-100 dark:bg-slate-800 px-2 py-0.5 rounded-full">
                         Disponível para configuração manual
                       </span>`;
                }
            } else {
                description = "Para jogos que não pertencem a uma biblioteca conhecida.";
                statusBadge = `<span class="inline-flex items-center gap-1.5 text-[10px] font-semibold text-emerald-700 dark:text-emerald-400 bg-emerald-100/70 dark:bg-emerald-900/50 px-2 py-0.5 rounded-full">
                     <span class="w-1.5 h-1.5 rounded-full bg-emerald-500"></span> Disponível para varredura
                   </span>`;
            }

            const pathDisplay = (lib.detected && lib.path)
                ? `<p class="text-[11px] text-slate-400 font-mono truncate mt-0.5">${escapeHtml(lib.path)}</p>`
                : "";

            html += `
                <div onclick="toggleOnboardingLibrary('${lib.id}')"
                     onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();toggleOnboardingLibrary('${lib.id}');}"
                     id="lib-card-${lib.id}"
                     tabindex="0"
                     role="checkbox"
                     aria-checked="${isSelected}"
                     class="onboarding-card p-4 rounded-2xl border-2 ${borderClass} cursor-pointer flex items-center justify-between gap-3 transition-all focus:outline-none">
                    <div class="flex items-center gap-3.5 min-w-0">
                        <div class="w-10 h-10 rounded-xl bg-slate-100 dark:bg-slate-800 flex items-center justify-center text-lg shrink-0">
                            ${icon}
                        </div>
                        <div class="min-w-0">
                            <div class="flex items-center gap-2">
                                <h4 class="text-sm font-bold text-slate-900 dark:text-slate-100">${escapeHtml(lib.name)}</h4>
                                ${statusBadge}
                            </div>
                            <p class="text-xs text-slate-500 dark:text-slate-400 mt-0.5">${escapeHtml(description)}</p>
                            ${pathDisplay}
                        </div>
                    </div>
                    <div id="lib-check-${lib.id}" class="w-6 h-6 rounded-full border-2 ${checkClass} flex items-center justify-center shrink-0">
                        ${checkContent}
                    </div>
                </div>
            `;
        });
        otherContainer.innerHTML = html;
    }
}

function toggleOnboardingLibrary(libId) {
    if (onboardingSelectedLibraries.has(libId)) {
        onboardingSelectedLibraries.delete(libId);
    } else {
        onboardingSelectedLibraries.add(libId);
    }
    renderOnboardingLibraries();
}

function renderOnboardingCloudFolders() {
    const container = document.getElementById("onboarding-cloud-folders-list");
    if (!container) return;

    if (onboardingDetectedCloudFolders.length === 0) {
        container.innerHTML = "";
        return;
    }

    let html = "";
    onboardingDetectedCloudFolders.forEach(df => {
        const isOneDrive = (df.name || "").toLowerCase().includes("onedrive");
        const isGoogleDrive = (df.name || "").toLowerCase().includes("google drive");
        const icon = isOneDrive ? "☁️" : (isGoogleDrive ? "🔷" : "📁");
        const providerKey = isOneDrive ? "onedrive" : (isGoogleDrive ? "google_drive" : "custom");
        const isSelected = onboardingStorageMode === providerKey && onboardingStoragePath === df.savevault_folder;

        const borderClass = isSelected
            ? "border-emerald-500 bg-emerald-50/20 dark:bg-emerald-950/20"
            : "border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40";
        const radioClass = isSelected
            ? "border-emerald-500 bg-emerald-500 text-white font-bold text-xs"
            : "border-slate-300 dark:border-slate-600";
        const radioContent = isSelected ? "✓" : "";

        const safeFolder = (df.savevault_folder || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
        const titleText = isOneDrive ? "OneDrive encontrado" : (isGoogleDrive ? "Google Drive encontrado" : `${escapeHtml(df.name)} encontrado`);
        const actionBadge = "Usar esta pasta";

        html += `
            <div onclick="selectOnboardingStorageMode('${providerKey}', '${safeFolder}')"
                 onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();selectOnboardingStorageMode('${providerKey}', '${safeFolder}');}"
                 tabindex="0"
                 role="radio"
                 aria-checked="${isSelected}"
                 class="onboarding-card p-4 rounded-2xl border-2 ${borderClass} cursor-pointer flex items-start gap-3.5 transition-all focus:outline-none">
                <div class="w-10 h-10 rounded-xl bg-blue-100 dark:bg-blue-900/40 text-blue-600 dark:text-blue-400 flex items-center justify-center text-lg shrink-0">
                    ${icon}
                </div>
                <div class="flex-1 min-w-0">
                    <div class="flex items-center justify-between">
                        <h4 class="text-sm font-bold text-slate-900 dark:text-slate-100">${titleText}</h4>
                        <span class="text-[10px] font-bold uppercase tracking-wider px-2 py-0.5 rounded-full bg-blue-100 dark:bg-blue-900/60 text-blue-800 dark:text-blue-300">${actionBadge}</span>
                    </div>
                    <p class="text-[11px] text-slate-500 dark:text-slate-400 font-mono truncate mt-1" title="${escapeHtml(df.savevault_folder)}">
                        ${escapeHtml(df.savevault_folder)}
                    </p>
                </div>
                <div class="w-5 h-5 rounded-full border-2 ${radioClass} flex items-center justify-center shrink-0 mt-0.5">
                    ${radioContent}
                </div>
            </div>
        `;
    });
    container.innerHTML = html;
}

function selectOnboardingStorageMode(mode, path = "") {
    onboardingStorageMode = mode;
    onboardingStoragePath = path;

    // Cartão Automático
    const cardAuto = document.getElementById("storage-card-automatic");
    const radioAuto = document.getElementById("storage-radio-automatic");
    if (cardAuto && radioAuto) {
        if (mode === "automatic") {
            cardAuto.className = "onboarding-card p-4 rounded-2xl border-2 border-emerald-500 bg-emerald-50/30 dark:bg-emerald-950/20 cursor-pointer flex items-start gap-3.5 transition-all focus:outline-none";
            cardAuto.setAttribute("aria-checked", "true");
            radioAuto.className = "w-5 h-5 rounded-full border-2 border-emerald-500 bg-emerald-500 text-white flex items-center justify-center text-xs shrink-0 mt-0.5";
            radioAuto.textContent = "✓";
        } else {
            cardAuto.className = "onboarding-card p-4 rounded-2xl border-2 border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40 cursor-pointer flex items-start gap-3.5 transition-all focus:outline-none";
            cardAuto.setAttribute("aria-checked", "false");
            radioAuto.className = "w-5 h-5 rounded-full border-2 border-slate-300 dark:border-slate-600 flex items-center justify-center text-xs shrink-0 mt-0.5";
            radioAuto.textContent = "";
        }
    }

    // Cartão Customizado
    const cardCustom = document.getElementById("storage-card-custom");
    const radioCustom = document.getElementById("storage-radio-custom");
    const customInputBox = document.getElementById("storage-custom-input-box");
    if (cardCustom && radioCustom) {
        if (mode === "custom") {
            cardCustom.className = "onboarding-card p-4 rounded-2xl border-2 border-emerald-500 bg-emerald-50/30 dark:bg-emerald-950/20 cursor-pointer transition-all focus:outline-none";
            cardCustom.setAttribute("aria-checked", "true");
            radioCustom.className = "w-5 h-5 rounded-full border-2 border-emerald-500 bg-emerald-500 text-white flex items-center justify-center text-xs shrink-0 mt-0.5";
            radioCustom.textContent = "✓";
            if (customInputBox) {
                customInputBox.classList.remove("hidden");
                document.getElementById("onboarding-custom-storage-input")?.focus();
            }
        } else {
            cardCustom.className = "onboarding-card p-4 rounded-2xl border-2 border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40 cursor-pointer transition-all focus:outline-none";
            cardCustom.setAttribute("aria-checked", "false");
            radioCustom.className = "w-5 h-5 rounded-full border-2 border-slate-300 dark:border-slate-600 flex items-center justify-center text-xs shrink-0 mt-0.5";
            radioCustom.textContent = "";
            if (customInputBox) {
                customInputBox.classList.add("hidden");
            }
        }
    }

    // Re-render nuvens para atualizar seleção se aplicável
    renderOnboardingCloudFolders();
}

async function browseCustomStorageFolder(event) {
    if (event) event.stopPropagation();
    try {
        dismissOnboardingError();
        let selectedPath = null;
        if (hasTauri && window.__TAURI__?.dialog?.open) {
            selectedPath = await window.__TAURI__.dialog.open({
                directory: true,
                multiple: false,
                title: "Selecione a pasta para os backups do SaveVault",
            });
        } else {
            selectedPath = await invoke("pick_folder");
        }

        if (selectedPath) {
            const inputEl = document.getElementById("onboarding-custom-storage-input");
            if (inputEl) {
                inputEl.value = selectedPath;
            }
            selectOnboardingStorageMode("custom", selectedPath);
        }
    } catch (err) {
        console.warn("Diálogo de pasta cancelado ou não suportado:", err);
    }
}

function selectOnboardingInterfaceMode(mode) {
    onboardingInterfaceMode = mode;

    const modes = ["basic", "advanced", "ultra_light"];
    modes.forEach(m => {
        const card = document.getElementById(`mode-card-${m}`);
        const radio = document.getElementById(`mode-radio-${m}`);
        if (!card || !radio) return;

        if (m === mode) {
            let activeColor = "emerald";
            if (m === "advanced") activeColor = "blue";
            if (m === "ultra_light") activeColor = "purple";

            card.className = `onboarding-card p-4.5 rounded-2xl border-2 border-${activeColor}-500 bg-${activeColor}-50/20 dark:bg-${activeColor}-950/20 cursor-pointer flex items-start gap-4 transition-all focus:outline-none`;
            card.setAttribute("aria-checked", "true");
            radio.className = `w-5 h-5 rounded-full border-2 border-${activeColor}-500 bg-${activeColor}-500 text-white flex items-center justify-center text-xs shrink-0 mt-0.5`;
            radio.textContent = "✓";
        } else {
            card.className = "onboarding-card p-4.5 rounded-2xl border-2 border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40 cursor-pointer flex items-start gap-4 transition-all focus:outline-none";
            card.setAttribute("aria-checked", "false");
            radio.className = "w-5 h-5 rounded-full border-2 border-slate-300 dark:border-slate-600 flex items-center justify-center text-xs shrink-0 mt-0.5";
            radio.textContent = "";
        }
    });
}

function onboardingAutostartToggleChanged() {
    const toggle = document.getElementById("onboarding-toggle-autostart");
    const label = document.getElementById("onboarding-autostart-label");
    if (toggle && label) {
        label.textContent = toggle.checked
            ? "Sim, iniciar com o computador"
            : "Não iniciar automaticamente";
    }
}

function onboardingGoToStep(step) {
    const isBack = step < currentOnboardingStep;
    currentOnboardingStep = step;

    // Subtítulo do indicador
    const stepTitles = [
        "Etapa 1 de 6 · Boas-vindas",
        "Etapa 2 de 6 · Bibliotecas",
        "Etapa 3 de 6 · Armazenamento",
        "Etapa 4 de 6 · Inicialização",
        "Etapa 5 de 6 · Modo de interface",
        "Etapa 6 de 6 · Revisão e conclusão"
    ];
    const indEl = document.getElementById("onboarding-step-indicator");
    if (indEl) {
        indEl.textContent = stepTitles[step - 1] || `Etapa ${step} de 6`;
    }

    // Atualiza Dots e Linhas
    for (let i = 1; i <= 6; i++) {
        const dot = document.getElementById(`step-dot-${i}`);
        if (dot) {
            if (i === step) {
                dot.className = "w-3 h-3 rounded-full bg-slate-900 dark:bg-emerald-500 ring-4 ring-emerald-500/20 transition-all duration-300";
            } else if (i < step) {
                dot.className = "w-2.5 h-2.5 rounded-full bg-emerald-500 text-white flex items-center justify-center transition-all duration-300";
            } else {
                dot.className = "w-2.5 h-2.5 rounded-full bg-slate-200 dark:bg-slate-800 transition-all duration-300";
            }
        }
        if (i < 6) {
            const line = document.getElementById(`step-line-${i}`);
            if (line) {
                if (i < step) {
                    line.className = "w-6 h-0.5 bg-emerald-500 transition-all duration-300";
                } else {
                    line.className = "w-6 h-0.5 bg-slate-200 dark:bg-slate-800 transition-all duration-300";
                }
            }
        }
    }

    // Alterna visibilidade dos Panes com transição suave de slide e fade
    for (let i = 1; i <= 6; i++) {
        const pane = document.getElementById(`onboarding-pane-${i}`);
        if (pane) {
            if (i === step) {
                pane.classList.remove("hidden");
                pane.classList.remove("onboarding-pane", "pane-back");
                void pane.offsetWidth; // Trigger reflow para reiniciar a animação
                pane.classList.add("onboarding-pane");
                if (isBack) {
                    pane.classList.add("pane-back");
                }
            } else {
                pane.classList.add("hidden");
                pane.classList.remove("onboarding-pane", "pane-back");
            }
        }
    }

    // Botões de navegação
    const prevBtn = document.getElementById("btn-onboarding-prev");
    const nextBtn = document.getElementById("btn-onboarding-next");
    const footer = document.getElementById("onboarding-footer");

    if (prevBtn) {
        if (step === 1) {
            prevBtn.classList.add("hidden");
        } else {
            prevBtn.classList.remove("hidden");
        }
    }

    if (footer) {
        if (step === 6) {
            footer.classList.add("hidden");
        } else {
            footer.classList.remove("hidden");
        }
    }

    if (nextBtn) {
        const labelEl = document.getElementById("btn-onboarding-next-label");
        if (step === 5) {
            if (labelEl) labelEl.textContent = "Revisar escolhas";
        } else {
            if (labelEl) labelEl.textContent = "Continuar";
        }
    }

    // Resumo final no passo 6
    if (step === 6) {
        prepareOnboardingSummary();
    }
}

function onboardingNextStep() {
    dismissOnboardingError();

    // Validações antes de avançar
    if (currentOnboardingStep === 3 && onboardingStorageMode === "custom") {
        const customPath = document.getElementById("onboarding-custom-storage-input")?.value?.trim();
        if (!customPath) {
            showOnboardingError("Por favor, digite o caminho da pasta desejada ou selecione 'Automático'.");
            return;
        }
    }

    if (currentOnboardingStep < 6) {
        onboardingGoToStep(currentOnboardingStep + 1);
    }
}

function onboardingPrevStep() {
    dismissOnboardingError();
    if (currentOnboardingStep > 1) {
        onboardingGoToStep(currentOnboardingStep - 1);
    }
}

function prepareOnboardingSummary() {
    // 1. Bibliotecas
    const libsSummary = document.getElementById("finish-summary-libs");
    if (libsSummary) {
        const names = [];
        if (onboardingSelectedLibraries.has("steam")) names.push("Steam");
        if (onboardingSelectedLibraries.has("gog")) names.push("GOG");
        if (onboardingSelectedLibraries.has("emulators")) names.push("Emuladores");
        if (onboardingSelectedLibraries.has("generic")) names.push("Genéricos");
        libsSummary.textContent = names.length > 0 ? names.join(", ") : "Nenhuma selecionada";
    }

    // 2. Armazenamento
    const storageSummary = document.getElementById("finish-summary-storage");
    if (storageSummary) {
        if (onboardingStorageMode === "automatic") {
            storageSummary.textContent = "Automático";
        } else if (onboardingStorageMode === "custom") {
            const customVal = document.getElementById("onboarding-custom-storage-input")?.value?.trim();
            storageSummary.textContent = customVal || "Pasta personalizada";
        } else if (onboardingStorageMode === "onedrive") {
            storageSummary.textContent = "OneDrive";
        } else {
            storageSummary.textContent = `${onboardingStorageMode.toUpperCase()}`;
        }
    }

    // 3. Inicialização
    const startupSummary = document.getElementById("finish-summary-startup");
    if (startupSummary) {
        const autostart = document.getElementById("onboarding-toggle-autostart")?.checked ?? true;
        startupSummary.textContent = autostart ? "Sim, com o computador" : "Não iniciar automaticamente";
    }

    // 4. Modo de Interface
    const modeSummary = document.getElementById("finish-summary-mode");
    if (modeSummary) {
        if (onboardingInterfaceMode === "basic") modeSummary.textContent = "Simples";
        else if (onboardingInterfaceMode === "advanced") modeSummary.textContent = "Avançado";
        else if (onboardingInterfaceMode === "ultra_light") modeSummary.textContent = "Ultra leve";
    }

    // Banner UltraLight
    const bannerUltraLight = document.getElementById("onboarding-ultralight-banner");
    if (bannerUltraLight) {
        if (onboardingInterfaceMode === "ultra_light") {
            bannerUltraLight.classList.remove("hidden");
        } else {
            bannerUltraLight.classList.add("hidden");
        }
    }

    // Garante que containers de resumo e animação estejam no estado inicial
    const summaryBox = document.getElementById("onboarding-summary-container");
    const prepBox = document.getElementById("onboarding-prep-container");
    if (summaryBox) summaryBox.classList.remove("hidden");
    if (prepBox) prepBox.classList.add("hidden");

    const finishTitle = document.getElementById("onboarding-finish-title");
    const finishStatus = document.getElementById("onboarding-finish-status-text");
    if (finishTitle) finishTitle.textContent = "Está tudo pronto.";
    if (finishStatus) finishStatus.textContent = "Confira o resumo das suas escolhas antes de concluir.";

    const finishBtn = document.getElementById("btn-onboarding-finish");
    const prevBtn = document.getElementById("btn-onboarding-finish-prev");
    if (finishBtn) {
        finishBtn.disabled = false;
        finishBtn.innerHTML = `Concluir configuração`;
    }
    if (prevBtn) {
        prevBtn.disabled = false;
    }
}

async function skipOnboarding() {
    try {
        await invoke("skip_onboarding");
        closeSetupWizard();
        const status = await invoke("get_setup_status");
        if (status && status.interface_mode) {
            applyInterfaceMode(status.interface_mode);
        } else {
            applyInterfaceMode("basic");
        }
        loadInicio();
        showToast("Configurações anteriores mantidas com sucesso!", "success");
    } catch (e) {
        console.warn("Erro ao registrar conclusão do onboarding:", e);
        closeSetupWizard();
    }
}

function formatHumanFriendlyError(err) {
    const raw = String(err?.message || err || "");
    if (raw.includes("Caminho não pode conter traversal") || raw.includes("..")) {
        return "O caminho informado contém caracteres inválidos ('..'). Escolha um diretório padrão.";
    }
    if (raw.includes("diretório raiz do disco") || raw.includes("raiz")) {
        return "Não é seguro utilizar a raiz do disco (ex: C:\\). Escolha uma subpasta dedicada aos seus saves.";
    }
    if (raw.includes("sistema operacional") || raw.includes("Windows") || raw.includes("Program Files")) {
        return "Essa pasta é protegida pelo sistema operacional. Por favor, selecione outra pasta para seus backups.";
    }
    if (raw.includes("permissão") || raw.includes("PermissionDenied") || raw.includes("AccessDenied")) {
        return "Não foi possível acessar a pasta selecionada. Verifique se o SaveVault possui permissão de leitura e gravação.";
    }
    if (raw.includes("autostart") || raw.includes("Registro")) {
        return "Não foi possível registrar a inicialização automática no sistema. Suas demais escolhas foram salvas.";
    }
    return `Não foi possível salvar sua configuração: ${raw}. Por favor, revise as opções e tente novamente.`;
}

async function finishOnboarding() {
    const finishBtn = document.getElementById("btn-onboarding-finish");
    const prevBtn = document.getElementById("btn-onboarding-finish-prev");
    dismissOnboardingError();

    let resolvedPath = null;
    if (onboardingStorageMode === "custom") {
        resolvedPath = document.getElementById("onboarding-custom-storage-input")?.value?.trim() || null;
        if (!resolvedPath) {
            showOnboardingError("Por favor, digite o caminho da pasta para guardar seus saves.");
            return;
        }
    } else if (onboardingStorageMode !== "automatic") {
        resolvedPath = onboardingStoragePath || null;
    }

    const autostart = document.getElementById("onboarding-toggle-autostart")?.checked ?? true;

    const reqPayload = {
        selected_libraries: Array.from(onboardingSelectedLibraries),
        detect_emulators: onboardingSelectedLibraries.has("emulators"),
        detect_generic_games: onboardingSelectedLibraries.has("generic"),
        storage_mode: onboardingStorageMode,
        storage_path: resolvedPath,
        startup_enabled: autostart,
        interface_mode: onboardingInterfaceMode,
    };

    if (finishBtn) {
        finishBtn.disabled = true;
        finishBtn.innerHTML = `<span class="animate-spin inline-block mr-1">🔄</span> Preparando...`;
    }
    if (prevBtn) prevBtn.disabled = true;

    // Alterna para tela de animação de preparação
    const summaryBox = document.getElementById("onboarding-summary-container");
    const prepBox = document.getElementById("onboarding-prep-container");
    const statusText = document.getElementById("onboarding-finish-status-text");

    if (summaryBox) summaryBox.classList.add("hidden");
    if (prepBox) prepBox.classList.remove("hidden");
    if (statusText) statusText.textContent = "Preparando o SaveVault…";

    // Reseta estado dos checks da animação
    for (let i = 1; i <= 4; i++) {
        const item = document.getElementById(`prep-item-${i}`);
        const check = document.getElementById(`prep-check-${i}`);
        if (item) item.className = "flex items-center gap-2.5 text-slate-400 dark:text-slate-500 transition-all duration-200";
        if (check) check.className = "w-5 h-5 rounded-full border border-slate-300 dark:border-slate-600 flex items-center justify-center text-[10px] text-transparent transition-all duration-200";
    }

    const markCheck = (id) => {
        const item = document.getElementById(`prep-item-${id}`);
        const check = document.getElementById(`prep-check-${id}`);
        if (item) item.className = "flex items-center gap-2.5 text-slate-900 dark:text-slate-100 font-medium transition-all duration-200";
        if (check) check.className = "w-5 h-5 rounded-full bg-emerald-500 border border-emerald-500 text-white font-bold flex items-center justify-center text-[10px] onboarding-pulse-check transition-all duration-200";
    };

    const isReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    try {
        // Passo 1: Preferências salvas
        markCheck(1);
        if (!isReducedMotion) await sleep(60);

        // Passo 2: Bibliotecas configuradas
        markCheck(2);

        // Envia requisição atômica ao backend (validação de paths no backend, persistência TOML, autostart)
        await invoke("complete_onboarding", { req: reqPayload });

        // Passo 3: Armazenamento preparado
        markCheck(3);
        if (!isReducedMotion) await sleep(60);

        // Passo 4: SaveVault pronto
        markCheck(4);
        if (statusText) statusText.textContent = "SaveVault pronto!";
        if (!isReducedMotion) await sleep(120);

        // Fecha modal do onboarding
        closeSetupWizard();

        // Aplica modo de interface
        applyInterfaceMode(onboardingInterfaceMode);

        // Feedback imediato
        showToast("SaveVault preparado com sucesso! Bem-vindo.", "success");
        loadInicio();

        // Dispara discovery assíncrono em background sem travar interface
        invoke("run_setup_discovery", { selectedLibraries: reqPayload.selected_libraries })
            .then(games => {
                if (Array.isArray(games) && games.length > 0) {
                    showToast(`${games.length} jogo(s) detectado(s) e prontos para proteção!`, "info");
                    loadInicio();
                    renderJogosList();
                }
            })
            .catch(err => {
                console.warn("Detecção inicial em background finalizada com aviso:", err);
            });

    } catch (e) {
        console.error("Falha ao concluir onboarding:", e);
        if (summaryBox) summaryBox.classList.remove("hidden");
        if (prepBox) prepBox.classList.add("hidden");
        if (statusText) statusText.textContent = "Confira o resumo das suas escolhas antes de concluir.";
        showOnboardingError(formatHumanFriendlyError(e));

        if (finishBtn) {
            finishBtn.disabled = false;
            finishBtn.innerHTML = `Tentar novamente`;
        }
        if (prevBtn) prevBtn.disabled = false;
    }
}

function applyInterfaceMode(mode) {
    const safeMode = mode || "basic";

    // 1. Atualiza badge de modo no dashboard se presente
    const modeBadge = document.getElementById("app-mode-badge");
    if (modeBadge) {
        modeBadge.classList.remove("hidden");
        if (safeMode === "basic") {
            modeBadge.textContent = "Modo: Simples";
            modeBadge.className = "inline-flex px-2.5 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wider bg-emerald-100 dark:bg-emerald-950/60 text-emerald-800 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800";
        } else if (safeMode === "advanced") {
            modeBadge.textContent = "Modo: Avançado";
            modeBadge.className = "inline-flex px-2.5 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wider bg-blue-100 dark:bg-blue-950/60 text-blue-800 dark:text-blue-300 border border-blue-200 dark:border-blue-800";
        } else if (safeMode === "ultra_light") {
            modeBadge.textContent = "Modo: Ultra Leve";
            modeBadge.className = "inline-flex px-2.5 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wider bg-purple-100 dark:bg-purple-950/60 text-purple-800 dark:text-purple-300 border border-purple-200 dark:border-purple-800";
        }
    }

    // 2. Adapta visibilidade de opções muito técnicas na aba de Configurações
    const maintenanceCards = document.querySelectorAll(".maintenance-technical-only");
    maintenanceCards.forEach(card => {
        if (safeMode === "basic") {
            card.classList.add("hidden");
        } else {
            card.classList.remove("hidden");
        }
    });

    // 3. Exibe aviso no dashboard se em Modo Ultra Leve
    const ultraLightNotice = document.getElementById("dashboard-ultralight-notice");
    if (ultraLightNotice) {
        if (safeMode === "ultra_light") {
            ultraLightNotice.classList.remove("hidden");
        } else {
            ultraLightNotice.classList.add("hidden");
        }
    }
}

// ── Init ──────────────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", () => {
    showTab("nav-inicio");
    loadInicio();
    checkInitialSetup();

    // Listener da barra de pesquisa de jogos
    const searchInput = document.getElementById("jogos-search-input");
    searchInput?.addEventListener("input", (e) => {
        renderJogosList(e.target.value);
    });
    searchInput?.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
            clearJogosSearch();
        }
    });
    document.getElementById("jogos-search-clear")?.addEventListener("click", () => {
        clearJogosSearch();
    });

    // Acessibilidade por teclado para cards interativos (Enter e Espaço)
    document.addEventListener("keydown", (e) => {
        if ((e.key === "Enter" || e.key === " ") && e.target && e.target.classList && e.target.classList.contains("onboarding-card")) {
            e.preventDefault();
            e.target.click();
        }
    });

    // Atualiza dashboard a cada 10s
    setInterval(loadInicio, 10000);
});
