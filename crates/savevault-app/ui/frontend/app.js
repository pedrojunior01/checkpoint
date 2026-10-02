const hasTauri = typeof window !== "undefined" && !!(window.__TAURI__ && window.__TAURI__.core);
const appWindow = (typeof window !== "undefined" && window.__TAURI__ && window.__TAURI__.window)
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
            return await res.json();
        }
        case "get_setup_status": {
            const res = await fetch("/api/setup/status");
            return await res.json();
        }
        case "run_setup_discovery": {
            const res = await fetch("/api/setup/scan", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
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
}

function renderJogosRecentes(locations) {
    const container = document.getElementById("jogos-recentes-list");
    if (!container) return;
    if (!locations || locations.length === 0) {
        container.innerHTML = `<p class="text-sm text-slate-400 px-1">Nenhum jogo detectado ainda.</p>`;
        return;
    }
    container.innerHTML = locations.map((loc, i) => {
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const grad = gradients[i % gradients.length];
        return `
        <div class="flex items-center gap-3 py-2.5 px-1 border-b border-slate-100 last:border-0">
            <div class="w-9 h-9 rounded-xl bg-gradient-to-br ${grad} flex items-center justify-center text-white text-xs font-bold flex-shrink-0">
                ${initials}
            </div>
            <div class="flex-1 min-w-0">
                <p class="text-sm font-semibold text-slate-800 truncate">${loc.game_name || "Jogo"}</p>
                <p class="text-xs text-slate-400">${loc.files_count ?? 0} arquivo(s) · ${timeAgo(loc.last_seen_at)}</p>
            </div>
            <span class="text-xs font-medium px-2 py-0.5 rounded-full bg-emerald-100 text-emerald-700 flex-shrink-0">Sincronizado</span>
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
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const grad = gradients[i % gradients.length];
        const safePath = (loc.path || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
        return `
        <div onclick="openGameDetails('${loc.id}')" class="bg-white dark:bg-slate-800/90 border border-slate-200/90 dark:border-slate-700/80 rounded-2xl p-4 flex items-center justify-between gap-4 shadow-xs hover:border-slate-300 dark:hover:border-slate-600 transition-colors cursor-pointer group">
            <div class="flex items-center gap-3.5 min-w-0">
                <div class="w-11 h-11 rounded-xl bg-gradient-to-br ${grad} flex items-center justify-center text-white text-sm font-bold flex-shrink-0 shadow-xs">
                    ${initials}
                </div>
                <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                        <p class="font-semibold text-slate-800 dark:text-slate-100 text-sm truncate">${escapeHtml(loc.game_name || "Jogo Desconhecido")}</p>
                        <span class="text-[10px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wider bg-slate-100 dark:bg-slate-700 text-slate-500 dark:text-slate-300 shrink-0">${escapeHtml(loc.platform || "PC")}</span>
                    </div>
                    <p class="text-xs text-slate-500 dark:text-slate-400 truncate mt-0.5 font-mono" title="${escapeHtml(loc.path || "")}">${escapeHtml(loc.path || "")}</p>
                    <p class="text-xs text-slate-400 dark:text-slate-500 mt-1">${loc.files_count ?? 0} arquivo(s) monitorado(s) · Visto ${timeAgo(loc.last_seen_at)}</p>
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
                <span class="text-xs font-medium px-2.5 py-1 rounded-full bg-emerald-100 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-400">Ativo</span>
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
    if (avatar) avatar.textContent = initials;

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
                await invoke("restore_snapshot", { snapshotId: id, snapshot_id: id });
                showToast(`Save de "${gameName}" restaurado com sucesso! Safety backup criado.`, "success");
                await loadAtividade();
                await loadInicio();
            } catch (e) {
                showToast(`Erro ao restaurar snapshot: ${e}`, "error");
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

// ── Assistente de Primeiro Acesso (Setup Wizard) ──────────────────────────────

let currentWizardStep = 1;
let wizardDiscoveredGames = [];

async function checkInitialSetup() {
    try {
        const status = await invoke("get_setup_status");
        if (status && !status.setup_completed) {
            openSetupWizard();
        }
    } catch (e) {
        console.warn("Não foi possível verificar status do setup inicial:", e);
    }
}

async function openSetupWizard() {
    currentWizardStep = 1;
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.remove("hidden");
    }
    wizardGoToStep(1);
    await loadWizardSetupStatus();
}

function closeSetupWizard() {
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.add("hidden");
    }
}

async function loadWizardSetupStatus() {
    try {
        const data = await invoke("get_setup_status");
        const foldersContainer = document.getElementById("wizard-detected-folders");
        const inputCloudPath = document.getElementById("wizard-input-cloud-path");

        if (foldersContainer && data.detected_cloud_folders) {
            if (data.detected_cloud_folders.length > 0) {
                let html = "";
                data.detected_cloud_folders.forEach(df => {
                    const safeFolder = (df.savevault_folder || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
                    const icon = (df.name || "").toLowerCase().includes("drive") ? "📁" : "☁️";
                    html += `
                        <div class="flex items-center justify-between p-2.5 bg-slate-50 border border-slate-200/90 rounded-xl text-xs">
                            <div class="flex items-center gap-2 overflow-hidden mr-2">
                                <span class="text-base">${icon}</span>
                                <div class="truncate">
                                    <p class="font-bold text-slate-800">${escapeHtml(df.name)}</p>
                                    <p class="text-[11px] text-slate-500 font-mono truncate">${escapeHtml(df.savevault_folder)}</p>
                                </div>
                            </div>
                            <button onclick="setWizardCloudPath('${safeFolder}')" class="px-2.5 py-1 bg-white hover:bg-slate-100 text-slate-700 font-semibold border border-slate-300/80 rounded-md transition-colors shrink-0">
                                Usar Esta
                            </button>
                        </div>
                    `;
                });
                foldersContainer.innerHTML = html;
                if (inputCloudPath && !inputCloudPath.value) {
                    inputCloudPath.value = data.local_sync_path || data.detected_cloud_folders[0].savevault_folder;
                }
            } else {
                foldersContainer.innerHTML = `
                    <div class="p-3 bg-amber-50 border border-amber-200 rounded-xl text-amber-800 text-[11px]">
                        Nenhuma pasta de nuvem padrão (Google Drive Desktop, OneDrive ou Dropbox) foi detectada automaticamente. Você pode digitar o caminho da sua pasta no campo abaixo.
                    </div>
                `;
            }
        }

        if (inputCloudPath && data.local_sync_path && !inputCloudPath.value) {
            inputCloudPath.value = data.local_sync_path;
        }

        const optAutostart = document.getElementById("wizard-opt-autostart");
        if (optAutostart && typeof data.autostart_enabled === "boolean") {
            optAutostart.checked = data.autostart_enabled;
        }

        const optNotifications = document.getElementById("wizard-opt-notifications");
        if (optNotifications && typeof data.notifications_enabled === "boolean") {
            optNotifications.checked = data.notifications_enabled;
        }
    } catch (e) {
        console.error("Erro ao carregar status do wizard:", e);
    }
}

function setWizardCloudPath(path) {
    const input = document.getElementById("wizard-input-cloud-path");
    if (input) {
        input.value = path;
    }
}

async function runWizardDiscovery() {
    const btn = document.getElementById("btn-wizard-scan");
    const container = document.getElementById("wizard-games-container");

    if (btn) {
        btn.disabled = true;
        btn.innerHTML = `<span class="animate-spin inline-block mr-1">🔄</span> Escaneando...`;
    }

    try {
        const games = await invoke("run_setup_discovery");
        wizardDiscoveredGames = games || [];

        if (container) {
            if (wizardDiscoveredGames.length === 0) {
                container.innerHTML = `
                    <div class="text-center py-6 text-slate-400 text-xs">
                        Nenhum save de jogo foi encontrado nas pastas padrão (Steam/GOG). Você poderá adicionar locais manualmente na aba "Saves" depois.
                    </div>
                `;
            } else {
                let html = "";
                wizardDiscoveredGames.forEach(g => {
                    html += `
                        <div class="p-2.5 bg-white border border-slate-200/90 rounded-lg text-xs flex items-center justify-between">
                            <div class="overflow-hidden mr-2">
                                <span class="font-bold text-slate-800">🎮 ${escapeHtml(g.name)}</span>
                                <span class="ml-1 text-[10px] uppercase font-bold text-slate-400 bg-slate-100 px-1.5 py-0.5 rounded">${escapeHtml(g.platform)}</span>
                                <p class="text-[11px] text-slate-500 font-mono truncate mt-0.5">${escapeHtml(g.save_path)}</p>
                            </div>
                            <span class="text-emerald-600 font-semibold text-[11px] shrink-0">Pronto</span>
                        </div>
                    `;
                });
                container.innerHTML = html;
            }
        }
    } catch (e) {
        if (container) {
            container.innerHTML = `<p class="text-xs text-red-500">Erro na detecção: ${e}</p>`;
        }
    } finally {
        if (btn) {
            btn.disabled = false;
            btn.innerHTML = `<span>🔍</span> Re-escanear`;
        }
    }
}

function wizardGoToStep(step) {
    currentWizardStep = step;

    const subtitles = [
        "Passo 1 de 5: Boas-vindas",
        "Passo 2 de 5: Pasta de Sincronização na Nuvem",
        "Passo 3 de 5: Varredura de Jogos",
        "Passo 4 de 5: Preferências do Sistema",
        "Passo 5 de 5: Conclusão e Ativação"
    ];

    const subEl = document.getElementById("wizard-step-subtitle");
    if (subEl) {
        subEl.textContent = subtitles[step - 1] || "";
    }

    for (let i = 1; i <= 5; i++) {
        const pill = document.getElementById(`pill-step-${i}`);
        if (pill) {
            if (i === step) {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-slate-800 text-white font-semibold";
            } else if (i < step) {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-emerald-100 text-emerald-800 font-semibold";
            } else {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-slate-100 text-slate-500";
            }
        }

        const pane = document.getElementById(`wizard-pane-${i}`);
        if (pane) {
            if (i === step) {
                pane.classList.remove("hidden");
            } else {
                pane.classList.add("hidden");
            }
        }
    }

    const prevBtn = document.getElementById("btn-wizard-prev");
    const nextBtn = document.getElementById("btn-wizard-next");

    if (prevBtn) {
        if (step === 1) {
            prevBtn.classList.add("hidden");
        } else {
            prevBtn.classList.remove("hidden");
        }
    }

    if (nextBtn) {
        if (step === 5) {
            nextBtn.innerHTML = `<span>Concluir e Proteger</span> 🚀`;
            nextBtn.className = "px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-semibold rounded-lg shadow-xs transition-colors flex items-center gap-1.5";
        } else {
            nextBtn.innerHTML = `<span>Avançar</span> →`;
            nextBtn.className = "px-4 py-2 bg-slate-800 hover:bg-slate-700 text-white text-xs font-semibold rounded-lg shadow-xs transition-colors flex items-center gap-1.5";
        }
    }

    if (step === 5) {
        const cloudPath = document.getElementById("wizard-input-cloud-path")?.value?.trim() || "Pasta padrão local";
        const sumCloud = document.getElementById("wizard-summary-cloud");
        if (sumCloud) sumCloud.textContent = cloudPath;

        const sumGames = document.getElementById("wizard-summary-games");
        if (sumGames) sumGames.textContent = `${wizardDiscoveredGames.length} jogo(s) detectado(s)`;

        const autostart = document.getElementById("wizard-opt-autostart")?.checked;
        const sumAutostart = document.getElementById("wizard-summary-autostart");
        if (sumAutostart) sumAutostart.textContent = autostart ? "Ativado" : "Desativado";

        const notif = document.getElementById("wizard-opt-notifications")?.checked;
        const sumNotif = document.getElementById("wizard-summary-notifications");
        if (sumNotif) sumNotif.textContent = notif ? "Ativadas" : "Desativadas";
    }
}

async function wizardNextStep() {
    if (currentWizardStep < 4) {
        wizardGoToStep(currentWizardStep + 1);
    } else if (currentWizardStep === 4) {
        const autostart = document.getElementById("wizard-opt-autostart")?.checked ?? false;
        const notifications = document.getElementById("wizard-opt-notifications")?.checked ?? true;
        const localSyncPath = document.getElementById("wizard-input-cloud-path")?.value?.trim() || null;

        try {
            await invoke("complete_setup", {
                provider: "local_folder",
                autostart,
                notifications,
                localSyncPath,
            });
        } catch (e) {
            console.error("Erro ao registrar conclusão do setup:", e);
        }
        wizardGoToStep(5);
    } else if (currentWizardStep === 5) {
        closeSetupWizard();
        loadInicio();
    }
}

function wizardPrevStep() {
    if (currentWizardStep > 1) {
        wizardGoToStep(currentWizardStep - 1);
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

    // Atualiza dashboard a cada 10s
    setInterval(loadInicio, 10000);
});
