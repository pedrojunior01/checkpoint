const { invoke } = window.__TAURI__.core;

// ── Utilitários ───────────────────────────────────────────────────────────────

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

function formatBytes(bytes) {
    if (!bytes || bytes === 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
}

// ── Notificações Flutuantes (Toast) ──────────────────────────────────────────

function showToast(message, type = "info") {
    const container = document.getElementById("toast-container");
    if (!container) return;
    const toast = document.createElement("div");
    toast.className = `toast ${type}`;
    const icon = type === "success" ? "✓" : type === "error" ? "✕" : "ℹ";
    toast.innerHTML = `<span style="font-weight:700;font-size:14px;">${icon}</span><span style="flex:1;">${message}</span>`;
    container.appendChild(toast);
    setTimeout(() => {
        toast.style.opacity = "0";
        toast.style.transform = "translateY(10px)";
        toast.style.transition = "all 0.3s ease";
        setTimeout(() => toast.remove(), 300);
    }, 4000);
}

// ── Diálogo Moderno de Confirmação In-App ─────────────────────────────────────

function showConfirmDialog(title, message) {
    return new Promise((resolve) => {
        const modal = document.getElementById("modal-confirm");
        const titleEl = document.getElementById("modal-confirm-title");
        const msgEl = document.getElementById("modal-confirm-message");
        const okBtn = document.getElementById("modal-confirm-ok");
        const cancelBtn = document.getElementById("modal-confirm-cancel");
        if (!modal) {
            resolve(window.confirm(`${title}\n\n${message}`));
            return;
        }
        titleEl.textContent = title;
        msgEl.textContent = message;
        modal.classList.remove("hidden");

        const cleanup = () => {
            modal.classList.add("hidden");
            okBtn.removeEventListener("click", onOk);
            cancelBtn.removeEventListener("click", onCancel);
        };
        const onOk = () => { cleanup(); resolve(true); };
        const onCancel = () => { cleanup(); resolve(false); };

        okBtn.addEventListener("click", onOk);
        cancelBtn.addEventListener("click", onCancel);
    });
}

// ── Tema Escuro / Claro (Dark Mode) ──────────────────────────────────────────

function initTheme() {
    const saved = localStorage.getItem("checkpoint_theme");
    const isDark = saved === "dark";
    setTheme(isDark);
}

function setTheme(dark) {
    const sunIcon = document.getElementById("theme-icon-sun");
    const moonIcon = document.getElementById("theme-icon-moon");
    if (dark) {
        document.body.classList.add("dark");
        if (sunIcon) sunIcon.style.display = "none";
        if (moonIcon) moonIcon.style.display = "block";
        localStorage.setItem("checkpoint_theme", "dark");
    } else {
        document.body.classList.remove("dark");
        if (sunIcon) sunIcon.style.display = "block";
        if (moonIcon) moonIcon.style.display = "none";
        localStorage.setItem("checkpoint_theme", "light");
    }
}

document.getElementById("btn-theme-toggle")?.addEventListener("click", () => {
    const isDark = document.body.classList.contains("dark");
    setTheme(!isDark);
});

// ── Central de Ajuda & Guia Rápido ───────────────────────────────────────────

document.getElementById("btn-open-help")?.addEventListener("click", () => {
    document.getElementById("modal-help")?.classList.remove("hidden");
});
document.getElementById("btn-close-help")?.addEventListener("click", () => {
    document.getElementById("modal-help")?.classList.add("hidden");
});
document.getElementById("btn-help-close")?.addEventListener("click", () => {
    document.getElementById("modal-help")?.classList.add("hidden");
});

// ── Controles da Janela (Nativos via IPC) ──────────────────────────────────────

const btnTitleMin = document.getElementById("titlebar-minimize");
const btnTitleMax = document.getElementById("titlebar-maximize");
const btnTitleClose = document.getElementById("titlebar-close");

[btnTitleMin, btnTitleMax, btnTitleClose].forEach(btn => {
    if (!btn) return;
    btn.addEventListener("pointerdown", (e) => e.stopPropagation());
    btn.addEventListener("mousedown", (e) => e.stopPropagation());
    btn.addEventListener("click", (e) => e.stopPropagation());
});

btnTitleMin?.addEventListener("click", () => {
    invoke("window_minimize").catch(console.error);
});
btnTitleMax?.addEventListener("click", () => {
    invoke("window_toggle_maximize").catch(console.error);
});
btnTitleClose?.addEventListener("click", () => {
    invoke("window_close").catch(console.error);
});

// Duplo clique na barra de título para maximizar/restaurar
document.querySelector(".titlebar-drag-spacer")?.addEventListener("dblclick", () => {
    invoke("window_toggle_maximize").catch(console.error);
});
document.querySelector(".titlebar-brand")?.addEventListener("dblclick", () => {
    invoke("window_toggle_maximize").catch(console.error);
});

// ── Navegação de Abas ─────────────────────────────────────────────────────────

const tabs = {
    "nav-inicio":       "tab-inicio",
    "nav-jogos":        "tab-jogos",
    "nav-atividade":    "tab-atividade",
    "nav-nuvem":        "tab-nuvem",
    "nav-config":       "tab-config",
};

function showTab(navId) {
    Object.values(tabs).forEach(tabId => {
        const el = document.getElementById(tabId);
        if (el) el.classList.add("hidden");
    });
    Object.keys(tabs).forEach(id => {
        const el = document.getElementById(id);
        if (el) el.classList.remove("active");
    });
    const tabId = tabs[navId];
    const tab = document.getElementById(tabId);
    if (tab) tab.classList.remove("hidden");
    const nav = document.getElementById(navId);
    if (nav) nav.classList.add("active");
}

Object.keys(tabs).forEach(navId => {
    document.getElementById(navId)?.addEventListener("click", (e) => {
        e.preventDefault();
        showTab(navId);
        if (navId === "nav-jogos") loadJogos();
        if (navId === "nav-atividade") loadAtividade();
        if (navId === "nav-nuvem") loadNuvem();
        if (navId === "nav-config") loadConfig();
    });
});

document.getElementById("link-ver-todos")?.addEventListener("click", (e) => {
    e.preventDefault();
    showTab("nav-jogos");
    loadJogos();
});

// ── Aba: Início ───────────────────────────────────────────────────────────────

// Ícones vetoriais SVG oficiais dos provedores de nuvem
function getGoogleDriveIcon(size = 18) {
    const height = Math.round(size * 78 / 87.3);
    return `<svg style="width:${size}px;height:${height}px;flex-shrink:0;vertical-align:middle;" viewBox="0 0 87.3 78" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="m6.6 66.85 3.85 6.65c.8 1.4 1.95 2.5 3.3 3.3l13.75-23.8h-27.5c0 1.55.4 3.1 1.2 4.5z" fill="#0066da"/>
        <path d="m43.65 25-13.75-23.8c-1.35.8-2.5 1.9-3.3 3.3l-25.4 44a9.06 9.06 0 0 0 -1.2 4.5h27.5z" fill="#00ac47"/>
        <path d="m73.55 76.8c1.35-.8 2.5-1.9 3.3-3.3l1.6-2.75 7.65-13.25c.8-1.4 1.2-2.95 1.2-4.5h-27.502l5.852 11.5z" fill="#ea4335"/>
        <path d="m43.65 25 13.75-23.8c-1.35-.8-2.9-1.2-4.5-1.2h-18.5c-1.6 0-3.15.45-4.5 1.2z" fill="#00832d"/>
        <path d="m59.8 53h-32.3l-13.75 23.8c1.35.8 2.9 1.2 4.5 1.2h50.8c1.6 0 3.15-.45 4.5-1.2z" fill="#2684fc"/>
        <path d="m73.4 26.5-12.7-22c-.8-1.4-1.95-2.5-3.3-3.3l-13.75 23.8 16.15 28h27.45c0-1.55-.4-3.1-1.2-4.5z" fill="#ffba00"/>
    </svg>`;
}

function getOneDriveIcon(size = 20) {
    const height = Math.round(size * 18 / 24);
    return `<svg style="width:${size}px;height:${height}px;flex-shrink:0;vertical-align:middle;" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M10.15 17.5H4.25A3.25 3.25 0 0 1 1 14.25a3.25 3.25 0 0 1 2.94-3.23A4.5 4.5 0 0 1 12.63 9.4a4.48 4.48 0 0 1 2.27 1.58A5 5 0 0 0 10.15 17.5z" fill="#0364B8"/>
        <path d="M18.5 17.5h-8.35a5 5 0 0 1 4.75-6.52c.6 0 1.18.1 1.72.3A4.25 4.25 0 0 1 18.5 9a4.25 4.25 0 0 1 4.25 4.25A4.25 4.25 0 0 1 18.5 17.5z" fill="#1490DF"/>
        <path d="M18.5 17.5h-8.35a4.98 4.98 0 0 1 2.68-4.42 4.24 4.24 0 0 1 2.07-.58 4.25 4.25 0 0 1 3.6 2 4.25 4.25 0 0 1 0 3z" fill="#28A8EA"/>
    </svg>`;
}

function getCloudWarnIcon(size = 18) {
    return `<svg style="width:${size}px;height:${size}px;flex-shrink:0;vertical-align:middle;color:#d97706;" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/></svg>`;
}

function identifyCloudProvider(providerType, path = "", name = "") {
    const p = (providerType || "").toLowerCase();
    const s = (path || "").toLowerCase();
    const n = (name || "").toLowerCase();

    // 1. OneDrive verificado primeiro para evitar falso-positivo com substring "drive"
    const isOneDrive = (p === "onedrive") || n.includes("onedrive") || s.includes("onedrive");
    if (isOneDrive) {
        return "onedrive";
    }

    // 2. Google Drive
    const isGDrive = (p === "google_drive")
        || n.includes("google")
        || n.includes("drive")
        || s.includes("google")
        || s.includes("meu drive")
        || s.includes("my drive")
        || s.startsWith("g:")
        || s.startsWith("h:");
    if (isGDrive) {
        return "google_drive";
    }

    if (p === "local_folder" || s.length > 0) {
        return "local_folder";
    }
    return "none";
}

// Resolução de imagem de capa quando a URL estática falha (404 ou erro de rede)
window.handleCoverError = async function(img, appId, gameName, initials, gradClass) {
    if (!img.dataset.retried) {
        img.dataset.retried = "1";
        try {
            const resolvedUrl = await invoke("resolve_game_cover", { 
                appId: appId || null, 
                gameName: gameName || null 
            });
            if (resolvedUrl && resolvedUrl !== img.src) {
                img.src = resolvedUrl;
                return;
            }
        } catch (_) {}
    }
    const isSmall = img.classList.contains("game-cover-art-sm");
    const styleAttr = isSmall ? 'style="width:38px;height:38px;"' : 'style="width:44px;height:44px;font-size:14px;"';
    img.outerHTML = `<div class="game-avatar ${gradClass}" ${styleAttr}>${initials}</div>`;
};

async function updateCloudBadge() {
    try {
        const cloudCfg = await invoke("get_cloud_config");
        cachedCloudConfig = cloudCfg;
        const badgeEl = document.getElementById("cloud-status-badge");
        const iconEl = document.getElementById("cloud-badge-icon");
        const textEl = document.getElementById("cloud-badge-text");
        if (!badgeEl) return;

        badgeEl.classList.remove("cloud-badge-onedrive", "cloud-badge-gdrive", "cloud-badge-warning");

        const identified = identifyCloudProvider(cloudCfg.provider, cloudCfg.local_sync_path);

        if (cloudCfg.enabled) {
            if (identified === "google_drive") {
                if (iconEl) iconEl.innerHTML = getGoogleDriveIcon(17);
                if (textEl) textEl.textContent = "Google Drive";
                badgeEl.classList.add("cloud-badge-gdrive");
                badgeEl.title = `Google Drive Conectado em: ${cloudCfg.local_sync_path}\nClique para gerenciar`;
            } else if (identified === "onedrive") {
                if (iconEl) iconEl.innerHTML = getOneDriveIcon(19);
                if (textEl) textEl.textContent = "OneDrive";
                badgeEl.classList.add("cloud-badge-onedrive");
                badgeEl.title = `Microsoft OneDrive Conectado em: ${cloudCfg.local_sync_path}\nClique para gerenciar`;
            } else {
                if (iconEl) iconEl.innerHTML = `<svg style="width:16px;height:16px;flex-shrink:0;vertical-align:middle;" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>`;
                if (textEl) textEl.textContent = "Pasta Local";
                badgeEl.title = `Sincronização ativa em: ${cloudCfg.local_sync_path}\nClique para gerenciar`;
            }
        } else {
            if (iconEl) iconEl.innerHTML = getCloudWarnIcon(17);
            if (textEl) textEl.textContent = "Nuvem Inativa";
            badgeEl.classList.add("cloud-badge-warning");
            badgeEl.title = "Clique para configurar a sincronização em nuvem";
        }
    } catch (e) {
        console.error("Erro ao atualizar badge de nuvem:", e);
    }
}

document.getElementById("cloud-status-badge")?.addEventListener("click", () => {
    showTab("nav-nuvem");
    loadNuvem();
});

async function loadInicio() {
    let latestLocBackup = null;
    try {
        const locs = await invoke("get_locations");
        cachedLocations = locs;
        latestLocBackup = locs.find(l => l.last_backup_time)?.last_backup_time;

        // Jogos recentes: prioriza jogos que possuem histórico de backup ou arquivos monitorados
        const comAtividade = locs.filter(l => l.last_backup_time || (l.monitored_files_count > 0));
        renderJogosRecentes((comAtividade.length > 0 ? comAtividade : locs).slice(0, 6));
    } catch (e) {
        console.error("Erro ao carregar jogos recentes:", e);
    }

    try {
        const data = await invoke("get_dashboard_status");

        const elGames = document.getElementById("kpi-games");
        const elSaves = document.getElementById("kpi-saves");
        const elTime = document.getElementById("kpi-time");
        if (elGames) elGames.textContent = data.save_locations_count ?? 0;
        if (elSaves) elSaves.textContent = data.snapshots_count ?? 0;
        if (elTime) {
            if (data.last_reconciliation_time) {
                elTime.textContent = formatHour(data.last_reconciliation_time);
            } else if (latestLocBackup) {
                elTime.textContent = timeAgo(latestLocBackup);
            } else {
                elTime.textContent = "—";
            }
        }

        const bannerText = document.getElementById("status-text");
        if (data.sync_pending_count > 0) {
            if (bannerText) bannerText.textContent = `${data.sync_pending_count} item(s) aguardando sincronização...`;
        } else {
            if (bannerText) bannerText.textContent = "Seus saves estão protegidos e sincronizados.";
        }
    } catch (e) {
        console.error("Erro ao carregar dashboard:", e);
    }

    updateCloudBadge();
}

function renderJogosRecentes(locations) {
    const container = document.getElementById("jogos-recentes-list");
    if (!container) return;
    if (!locations || locations.length === 0) {
        container.innerHTML = `
        <div style="text-align:center;padding:26px 16px;background:rgba(255,255,255,0.02);border:1px dashed var(--border-color, #334155);border-radius:12px;">
            <div style="font-size:28px;margin-bottom:8px;">🎮</div>
            <h3 style="font-size:15px;font-weight:600;margin-bottom:6px;color:var(--text-primary, #f1f5f9);">Nenhum jogo monitorado no momento</h3>
            <p style="font-size:13px;color:#94a3b8;max-width:440px;margin:0 auto 16px auto;">O Checkpoint busca jogos instalados da Steam, GOG e pastas locais de saves. Clique no botão abaixo para escanear seu computador.</p>
            <div style="display:flex;justify-content:center;gap:10px;flex-wrap:wrap;">
                <button onclick="window.triggerScanGames()" class="btn-primary" style="display:inline-flex;align-items:center;gap:8px;padding:8px 18px;font-size:13px;">
                    <svg width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/></svg>
                    <span>Escanear Jogos Agora</span>
                </button>
                <button onclick="window.openAddGameModal()" class="btn-secondary" style="display:inline-flex;align-items:center;gap:6px;padding:8px 14px;font-size:13px;">
                    <span>+ Adicionar Manualmente</span>
                </button>
            </div>
        </div>`;
        return;
    }
    // Jogos encontrados — reseta o contador de auto-refresh
    window._discoveryAutoRefreshCount = 0;

    container.innerHTML = locations.map((loc, i) => {

        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const gradClass = `grad-${i % 6}`;
        const appId = (loc.cover_url?.match(/\/apps\/(\d+)\//)?.[1]) || "";
        const safeGameName = (loc.game_name || "").replace(/'/g, "\\'");
        const avatarHtml = loc.cover_url
            ? `<img src="${loc.cover_url}" class="game-cover-art game-cover-art-sm" style="width:38px;height:38px;" onerror="window.handleCoverError(this, '${appId}', '${safeGameName}', '${initials}', '${gradClass}')" alt="${loc.game_name}" />`
            : `<div class="game-avatar ${gradClass}" data-loc-id="${loc.id}" style="width:38px;height:38px;">${initials}</div>`;

        const timeSubtext = loc.last_backup_time ? `Backup ${timeAgo(loc.last_backup_time)}` : (loc.platform || "PC");
        const statusBadge = loc.last_backup_time
            ? `<span class="badge-synced" title="Último backup: ${loc.last_backup_time}">Protegido</span>`
            : `<span class="badge-synced">Ativo</span>`;

        return `
        <div class="game-item game-card-clickable" onclick="window.openGameDetails('${loc.id}')">
            ${avatarHtml}
            <div class="game-info">
                <p class="game-name">${loc.game_name || "Jogo"}</p>
                <p class="game-subtext">${loc.monitored_files_count ?? 0} arquivo(s) · ${timeSubtext}</p>
            </div>
            ${statusBadge}
        </div>`;
    }).join("");

    // Resolução dinâmica em segundo plano para jogos que ainda não possuem URL estática
    locations.forEach((loc, i) => {
        if (!loc.cover_url && loc.game_name) {
            const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
            const gradClass = `grad-${i % 6}`;
            const safeGameName = (loc.game_name || "").replace(/'/g, "\\'");
            invoke("resolve_game_cover", { appId: null, gameName: loc.game_name }).then(url => {
                if (url) {
                    loc.cover_url = url;
                    const avatarEl = container.querySelector(`.game-avatar[data-loc-id="${loc.id}"]`);
                    if (avatarEl) {
                        const img = document.createElement("img");
                        img.src = url;
                        img.className = "game-cover-art game-cover-art-sm";
                        img.style.width = "38px";
                        img.style.height = "38px";
                        img.alt = loc.game_name;
                        img.onerror = () => window.handleCoverError(img, "", safeGameName, initials, gradClass);
                        avatarEl.replaceWith(img);
                    }
                }
            }).catch(() => {});
        }
    });
}

// ── Aba: Jogos ────────────────────────────────────────────────────────────────

let cachedLocations = [];

async function loadJogos() {
    const container = document.getElementById("jogos-list");
    if (!container) return;
    container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Carregando jogos...</p>`;
    try {
        cachedLocations = await invoke("get_locations");
        const query = document.getElementById("jogos-search-input")?.value?.trim() || "";
        renderJogosList(query);
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro ao carregar jogos: ${e}</p>`;
    }
}

function renderJogosList(query = "") {
    const container = document.getElementById("jogos-list");
    if (!container) return;

    const lowerQuery = query.toLowerCase();
    const filtered = cachedLocations.filter(loc => {
        const name = (loc.game_name || "").toLowerCase();
        const path = (loc.path || "").toLowerCase();
        return name.includes(lowerQuery) || path.includes(lowerQuery);
    });

    if (filtered.length === 0) {
        if (query) {
            container.innerHTML = `<p style="font-size:13px;color:#94a3b8;padding:12px 0;">Nenhum jogo encontrado para "${query}".</p>`;
        } else {
            container.innerHTML = `
            <div style="text-align:center;padding:36px 16px;background:rgba(255,255,255,0.03);border:1px dashed #cbd5e1;border-radius:12px;margin-top:8px;">
                <p style="font-size:14px;font-weight:600;color:#64748b;margin-bottom:6px;">Nenhum jogo detectado nesta máquina</p>
                <p style="font-size:12.5px;color:#94a3b8;margin-bottom:16px;">Você pode fazer uma varredura automática agora ou adicionar uma pasta de save manualmente.</p>
                <div style="display:inline-flex;gap:10px;">
                    <button onclick="window.triggerScanGames()" class="btn-primary" style="display:inline-flex;align-items:center;gap:6px;font-size:13px;padding:8px 14px;">
                        <svg width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/></svg>
                        <span>Escanear Jogos Instalados</span>
                    </button>
                    <button onclick="document.getElementById('btn-open-add-game')?.click()" class="btn-secondary" style="font-size:13px;padding:8px 14px;">
                        + Adicionar Pasta
                    </button>
                </div>
            </div>`;
        }
        return;
    }

    container.innerHTML = filtered.map((loc, i) => {
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const gradClass = `grad-${i % 6}`;
        const appId = (loc.cover_url?.match(/\/apps\/(\d+)\//)?.[1]) || "";
        const safeGameName = (loc.game_name || "").replace(/'/g, "\\'");
        const avatarHtml = loc.cover_url
            ? `<img src="${loc.cover_url}" class="game-cover-art" onerror="window.handleCoverError(this, '${appId}', '${safeGameName}', '${initials}', '${gradClass}')" alt="${loc.game_name}" />`
            : `<div class="game-avatar ${gradClass}" data-loc-id="${loc.id}" style="width:44px;height:44px;font-size:14px;">${initials}</div>`;

        return `
        <div class="content-card game-card-clickable" style="display:flex;align-items:center;gap:14px;margin-bottom:10px;padding:14px;" onclick="window.openGameDetails('${loc.id}')">
            ${avatarHtml}
            <div class="game-info">
                <p class="game-name" style="font-size:14px;">${loc.game_name || "Jogo Desconhecido"}</p>
                <p class="game-subtext game-path-text">${loc.path || ""}</p>
                <p class="game-subtext">${loc.monitored_files_count ?? 0} arquivo(s) monitorado(s) · Plataforma: ${loc.platform || "Genérica"}${loc.last_backup_time ? ` · Backup ${timeAgo(loc.last_backup_time)}` : ""}</p>
            </div>
            <div style="display:flex;align-items:center;gap:8px;">
                <span class="badge-synced">Monitorado</span>
                <button class="btn-secondary" style="padding:5px 8px;font-size:12px;color:#ef4444;border-color:rgba(239,68,68,0.25);display:inline-flex;align-items:center;" title="Remover jogo do monitoramento" onclick="event.stopPropagation(); window.confirmDeleteGame('${loc.id}', '${safeGameName}')">
                    <svg width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg>
                </button>
            </div>
        </div>`;
    }).join("");

    // Resolução dinâmica em segundo plano para jogos da lista geral
    filtered.forEach((loc, i) => {
        if (!loc.cover_url && loc.game_name) {
            const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
            const gradClass = `grad-${i % 6}`;
            const safeGameName = (loc.game_name || "").replace(/'/g, "\\'");
            invoke("resolve_game_cover", { appId: null, gameName: loc.game_name }).then(url => {
                if (url) {
                    loc.cover_url = url;
                    const avatarEl = container.querySelector(`.game-avatar[data-loc-id="${loc.id}"]`);
                    if (avatarEl) {
                        const img = document.createElement("img");
                        img.src = url;
                        img.className = "game-cover-art";
                        img.alt = loc.game_name;
                        img.onerror = () => window.handleCoverError(img, "", safeGameName, initials, gradClass);
                        avatarEl.replaceWith(img);
                    }
                }
            }).catch(() => {});
        }
    });
}

// Filtro em tempo real no campo de busca
document.getElementById("jogos-search-input")?.addEventListener("input", (e) => {
    renderJogosList(e.target.value.trim());
});

window.openAddGameModal = function() {
    const nameInput = document.getElementById("add-game-name");
    const pathInput = document.getElementById("add-game-path");
    if (nameInput) nameInput.value = "";
    if (pathInput) pathInput.value = "";
    document.getElementById("modal-add-game")?.classList.remove("hidden");
    if (nameInput) nameInput.focus();
};

window.triggerScanGames = async function() {
    const buttons = [
        document.getElementById("btn-scan-games"),
        document.getElementById("btn-scan-games-recentes"),
    ].filter(Boolean);

    const savedState = buttons.map(b => ({ btn: b, html: b.innerHTML }));
    buttons.forEach(b => {
        b.disabled = true;
        b.innerHTML = `<span style="width:13px;height:13px;border:2px solid currentColor;border-top-color:transparent;border-radius:50%;display:inline-block;animation:spin 0.8s linear infinite;"></span><span>Buscando...</span>`;
    });

    showToast("Varrendo Steam, GOG e pastas locais por jogos...", "info");
    try {
        const res = await invoke("scan_games");
        showToast(`Varredura concluída! ${res.games_count} jogo(s) detectado(s), ${res.locations_count} local(is) monitorado(s).`, "success");
        await loadJogos();
        await loadInicio();
    } catch (err) {
        showToast(`Erro ao escanear jogos: ${err}`, "error");
    } finally {
        savedState.forEach(s => {
            s.btn.disabled = false;
            s.btn.innerHTML = s.html;
        });
    }
};

document.getElementById("btn-scan-games")?.addEventListener("click", () => {
    window.triggerScanGames();
});
document.getElementById("btn-scan-games-recentes")?.addEventListener("click", () => {
    window.triggerScanGames();
});

document.getElementById("btn-clean-games")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-clean-games");
    const originalHtml = btn ? btn.innerHTML : "";
    if (btn) {
        btn.disabled = true;
        btn.innerHTML = `<span style="width:13px;height:13px;border:2px solid currentColor;border-top-color:transparent;border-radius:50%;display:inline-block;animation:spin 0.8s linear infinite;"></span><span>Limpando...</span>`;
    }
    try {
        const res = await invoke("clean_unrecognized_games");
        if (res.locations_removed > 0 || res.games_removed > 0) {
            showToast(`Limpeza concluída! ${res.games_removed} jogo(s) e ${res.locations_removed} local(is) não reconhecidos foram removidos.`, "success");
        } else {
            showToast("Nenhum registro antigo ou órfão encontrado para remover.", "info");
        }
        await loadJogos();
        await loadInicio();
    } catch (err) {
        showToast(`Erro na limpeza: ${err}`, "error");
    } finally {
        if (btn) {
            btn.disabled = false;
            btn.innerHTML = originalHtml;
        }
    }
});


// ── Modal: Detalhes do Jogo ──────────────────────────────────────────────────

window.openGameDetails = function(locationId) {
    const loc = cachedLocations.find(l => String(l.id) === String(locationId));
    if (!loc) return;

    const modal = document.getElementById("modal-game-details");
    const titleEl = document.getElementById("modal-details-title");
    const pathEl = document.getElementById("modal-details-path");
    const filesEl = document.getElementById("modal-details-files");
    const diskEl = document.getElementById("modal-details-disk");
    const backupEl = document.getElementById("modal-details-backup");
    const openBtn = document.getElementById("btn-details-open-folder");

    if (titleEl) titleEl.textContent = loc.game_name || "Detalhes do Jogo";
    if (pathEl) pathEl.textContent = loc.path || "—";
    if (filesEl) filesEl.textContent = `${loc.monitored_files_count ?? 0} arquivo(s)`;
    if (backupEl) backupEl.textContent = loc.last_backup_time ? timeAgo(loc.last_backup_time) : "Nenhum";
    if (diskEl) {
        diskEl.textContent = loc.exists_on_disk ? "Detectado no Disco" : "Caminho Não Encontrado";
        diskEl.style.color = loc.exists_on_disk ? "#10b981" : "#ef4444";
    }

    if (openBtn) {
        openBtn.onclick = () => {
            invoke("open_folder", { path: loc.path }).catch(err => {
                showToast(`Erro ao abrir pasta: ${err}`, "error");
            });
        };
    }

    const deleteBtn = document.getElementById("btn-details-delete-game");
    if (deleteBtn) {
        deleteBtn.onclick = () => {
            window.confirmDeleteGame(loc.id, loc.game_name);
        };
    }

    const btnRestoreLatest = document.getElementById("btn-details-restore-latest");
    const btnRestoreDaily = document.getElementById("btn-details-restore-daily");
    if (btnRestoreLatest) {
        btnRestoreLatest.onclick = () => window.handleRestoreGame(loc.id, loc.game_name, "latest");
    }
    if (btnRestoreDaily) {
        btnRestoreDaily.onclick = () => window.handleRestoreGame(loc.id, loc.game_name, "daily");
    }

    modal?.classList.remove("hidden");
};

window.handleRestoreGame = async function(locationId, gameName, targetType) {
    const isDaily = targetType === "daily";
    const typeLabel = isDaily ? "Save Diário (Daily)" : "Último Save (Latest)";
    const typeDesc = isDaily 
        ? "Você está prestes a reverter o jogo para o marco congelado no início do dia. Isso desfará qualquer problema ou alteração ocorrida durante as sessões de hoje."
        : "Você está prestes a restaurar a cópia mais recente salva pelo jogo.";

    const confirmed = await showConfirmDialog(
        `Restaurar: ${typeLabel}`,
        `${typeDesc}\n\nJogo: "${gameName}"\n\nFique tranquilo: um Safety Backup do seu save atual será criado antes da substituição.`
    );
    if (!confirmed) return;

    try {
        const result = await invoke("restore_game_saves", { locationId: String(locationId), targetType });
        if (result.success) {
            showToast(`Restauração de "${gameName}" (${typeLabel}) concluída com sucesso!`, "success");
            document.getElementById("modal-game-details")?.classList.add("hidden");
            await loadInicio();
            await loadJogos();
            await loadAtividade();
        } else {
            showToast(`Falha na restauração: ${result.error || "Erro desconhecido"}`, "error");
        }
    } catch (e) {
        showToast(`Erro ao restaurar: ${e}`, "error");
    }
};

window.confirmDeleteGame = async function(locationId, gameName) {
    const displayName = gameName || "este jogo";
    if (!confirm(`Deseja remover "${displayName}" do checkpoint?\n\nEste jogo deixará de ser monitorado e seus saves não serão sincronizados.`)) {
        return;
    }
    try {
        await invoke("remove_game_location", { locationId: String(locationId) });
        showToast(`"${displayName}" foi removido do monitoramento.`, "success");
        document.getElementById("modal-game-details")?.classList.add("hidden");
        await loadJogos();
        await loadInicio();
    } catch (err) {
        showToast(`Erro ao remover jogo: ${err}`, "error");
    }
};

document.getElementById("btn-close-details")?.addEventListener("click", () => {
    document.getElementById("modal-game-details")?.classList.add("hidden");
});
document.getElementById("btn-details-close")?.addEventListener("click", () => {
    document.getElementById("modal-game-details")?.classList.add("hidden");
});

// ── Modal: Adicionar Jogo Manualmente ────────────────────────────────────────

document.getElementById("btn-open-add-game")?.addEventListener("click", () => {
    const nameInput = document.getElementById("add-game-name");
    const pathInput = document.getElementById("add-game-path");
    if (nameInput) nameInput.value = "";
    if (pathInput) pathInput.value = "";
    document.getElementById("modal-add-game")?.classList.remove("hidden");
    nameInput?.focus();
});

document.getElementById("btn-close-add-game")?.addEventListener("click", () => {
    document.getElementById("modal-add-game")?.classList.add("hidden");
});
document.getElementById("btn-cancel-add-game")?.addEventListener("click", () => {
    document.getElementById("modal-add-game")?.classList.add("hidden");
});

document.getElementById("btn-submit-add-game")?.addEventListener("click", async () => {
    const name = document.getElementById("add-game-name")?.value?.trim();
    const path = document.getElementById("add-game-path")?.value?.trim();

    if (!name || !path) {
        showToast("Por favor, preencha o nome do jogo e o caminho da pasta.", "error");
        return;
    }

    const btn = document.getElementById("btn-submit-add-game");
    if (btn) {
        btn.textContent = "Adicionando...";
        btn.disabled = true;
    }

    try {
        await invoke("add_game", { name, path });
        showToast(`Jogo "${name}" adicionado com sucesso!`, "success");
        document.getElementById("modal-add-game")?.classList.add("hidden");
        loadJogos();
    } catch (e) {
        showToast(`Erro ao adicionar jogo: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "Adicionar Jogo";
            btn.disabled = false;
        }
    }
});

// ── Aba: Atividade ────────────────────────────────────────────────────────────

let cachedSnapshots = [];

async function loadAtividade() {
    const container = document.getElementById("atividade-list");
    if (!container) return;
    container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Carregando histórico...</p>`;
    try {
        cachedSnapshots = await invoke("get_snapshots");
        populateAtividadeGameFilter();
        renderAtividadeList();
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro: ${e}</p>`;
    }
}

function populateAtividadeGameFilter() {
    const filterSelect = document.getElementById("atividade-game-filter");
    if (!filterSelect) return;

    const currentVal = filterSelect.value;
    const uniqueGames = [...new Set(cachedSnapshots.map(s => s.game_name).filter(Boolean))].sort();

    filterSelect.innerHTML = `<option value="">Todos os jogos (${cachedSnapshots.length})</option>` +
        uniqueGames.map(g => `<option value="${g}">${g}</option>`).join("");

    if (uniqueGames.includes(currentVal)) {
        filterSelect.value = currentVal;
    }
}

function renderAtividadeList() {
    const container = document.getElementById("atividade-list");
    if (!container) return;

    const filterVal = document.getElementById("atividade-game-filter")?.value || "";
    const filtered = filterVal
        ? cachedSnapshots.filter(s => s.game_name === filterVal)
        : cachedSnapshots;

    if (!filtered || filtered.length === 0) {
        container.innerHTML = `<p style="font-size:13px;color:#94a3b8;padding:12px 0;">Nenhum snapshot encontrado para os critérios selecionados.</p>`;
        return;
    }

    container.innerHTML = filtered.map(s => `
        <div class="activity-item">
            <div class="dot-indicator"></div>
            <div class="game-info">
                <p class="game-name">${s.game_name || "Arquivo de Save"}</p>
                <p class="game-subtext" style="font-family:ui-monospace, monospace;">${s.file_name || s.short_hash || ""}</p>
                <p class="game-subtext">${s.size_formatted || formatBytes(s.size_bytes)} · Versão ${s.version} · ${timeAgo(s.created_at)}</p>
            </div>
            <div style="display:flex;align-items:center;gap:6px;">
                <button
                    onclick="window.handleRestoreSnapshot('${s.id}', '${(s.game_name || "").replace(/'/g, "")}', ${s.version}, 'latest')"
                    class="btn-restore" title="Restaurar a partir do Último Save (Latest)">
                    Latest
                </button>
                <button
                    onclick="window.handleRestoreSnapshot('${s.id}', '${(s.game_name || "").replace(/'/g, "")}', ${s.version}, 'daily')"
                    class="btn-secondary" style="font-size:12px;padding:5px 9px;" title="Restaurar a partir do Ponto Diário (Daily)">
                    Daily
                </button>
            </div>
        </div>`).join("");
}

document.getElementById("atividade-game-filter")?.addEventListener("change", () => {
    renderAtividadeList();
});

window.handleRestoreSnapshot = async function(id, gameName, version, targetType = "latest") {
    const isDaily = targetType === "daily";
    const typeLabel = isDaily ? "Marco Diário (Daily - Início do Dia)" : "Último Save (Latest - Tempo Real)";
    const confirmed = await showConfirmDialog(
        `Restaurar: ${typeLabel}`,
        `Deseja restaurar a versão ${version} do save de "${gameName}" usando ${typeLabel}?\n\n${isDaily ? 'Isso reverterá para a cópia segura do início do dia.' : 'Isso usará a cópia mais recente gravada.'}\nFique tranquilo: um Safety Backup do save ativo atual será criado antes da substituição.`
    );
    if (!confirmed) return;

    try {
        const result = await invoke("restore_snapshot", { snapshotId: id, targetType: targetType || null });
        if (result.success) {
            showToast(`Restauração concluída! Save revertido usando ${typeLabel}.`, "success");
            loadAtividade();
        } else {
            showToast(`Falha na restauração: ${result.error || "Erro desconhecido"}`, "error");
        }
    } catch (e) {
        showToast(`Erro ao restaurar: ${e}`, "error");
    }
};

// ── Aba: Nuvem ────────────────────────────────────────────────────────────────

let cachedCloudConfig = null;

async function loadNuvem() {
    const container = document.getElementById("nuvem-info");
    if (!container) return;
    try {
        cachedCloudConfig = await invoke("get_cloud_config");
        const prov = cachedCloudConfig.provider;
        const localPath = cachedCloudConfig.local_sync_path || "";
        const identified = identifyCloudProvider(prov, localPath);

        const provName = identified === "onedrive" ? "Microsoft OneDrive" :
                         identified === "google_drive" ? "Google Drive" :
                         prov === "local_folder" ? "Pasta Sincronizada Local" : "Nenhum (Desativado)";

        const isConnected = cachedCloudConfig.enabled;
        const iconBg = isConnected 
            ? (identified === "onedrive" ? "rgba(0,120,212,0.12)" : "rgba(0,172,71,0.12)") 
            : "rgba(100,116,139,0.12)";
        const iconHtml = identified === "google_drive" ? getGoogleDriveIcon(26) 
                       : identified === "onedrive" ? getOneDriveIcon(28) 
                       : "📁";

        const oneDriveDetected = (cachedCloudConfig.detected_folders || []).find(
            f => (f.provider_type === "onedrive") || (f.name || "").toLowerCase().includes("onedrive")
        );
        const oneDriveFolder = oneDriveDetected ? (oneDriveDetected.savevault_folder || oneDriveDetected.sync_folder) : null;

        const quickOneDriveButton = (!isConnected && oneDriveFolder) ? `
            <div class="cloud-detected-alert" style="margin-top:12px;padding:12px 14px;border-radius:10px;display:flex;align-items:center;justify-content:space-between;">
                <div style="display:flex;align-items:center;gap:10px;">
                    ${getOneDriveIcon(22)}
                    <div>
                        <p style="font-weight:600;font-size:13px;">Microsoft OneDrive detectado no seu PC!</p>
                        <p style="font-size:11.5px;font-family:ui-monospace, monospace;margin-top:2px;">${oneDriveFolder}</p>
                    </div>
                </div>
                <button type="button" class="btn-primary" style="font-size:12px;padding:6px 14px;" onclick="window.quickEnableOneDrive('${oneDriveFolder.replace(/\\/g, "\\\\")}')">
                    Ativar OneDrive Agora
                </button>
            </div>
        ` : "";

        container.innerHTML = `
            <div class="content-card" style="display:flex;flex-direction:column;gap:14px;">
                <div style="display:flex;align-items:center;justify-content:space-between;">
                    <div style="display:flex;align-items:center;gap:12px;">
                        <div style="width:42px;height:42px;border-radius:10px;background:${iconBg};display:flex;align-items:center;justify-content:center;">${iconHtml}</div>
                        <div>
                            <p style="font-size:14px;font-weight:600;" class="game-name">${provName}</p>
                            <p style="font-size:12px;color:${isConnected ? "#10b981" : "#64748b"};font-weight:500;">
                                ${isConnected ? "● Sincronização Ativa em Tempo Real" : "○ Sincronização Desativada"}
                            </p>
                        </div>
                    </div>
                    <button onclick="window.openCloudModal()" class="btn-secondary">Alterar Provedor</button>
                </div>
                ${quickOneDriveButton}
                <div class="cloud-details-footer" style="font-size:12.5px;padding-top:12px;line-height:1.7;">
                    <p>📂 Diretório de Sincronização: <strong style="font-family:ui-monospace, monospace;font-size:12px;">${cachedCloudConfig.local_sync_path || "Nenhuma pasta configurada"}</strong></p>
                    <p>🔍 Serviços Detectados no Sistema: <strong>${cachedCloudConfig.detected_folders?.length ?? 0} serviço(s)</strong></p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro ao carregar status da nuvem: ${e}</p>`;
    }
}

window.quickEnableOneDrive = async function(folderPath) {
    try {
        await invoke("update_cloud_config", {
            provider: "onedrive",
            localSyncPath: folderPath,
            enabled: true,
        });
        showToast("Sincronização com Microsoft OneDrive ativada!", "success");
        await loadNuvem();
        await updateCloudBadge();
    } catch (e) {
        showToast(`Erro ao ativar OneDrive: ${e}`, "error");
    }
};

window.openCloudModal = function() {
    const modal = document.getElementById("modal-cloud-config");
    const listEl = document.getElementById("cloud-detected-list");
    const customInput = document.getElementById("cloud-custom-path");

    if (customInput && cachedCloudConfig) {
        customInput.value = cachedCloudConfig.local_sync_path || "";
    }

    if (listEl && cachedCloudConfig) {
        const folders = cachedCloudConfig.detected_folders || [];
        if (folders.length === 0) {
            listEl.innerHTML = `<p style="font-size:12px;color:#94a3b8;">Nenhum cliente de nuvem padrão (OneDrive/Drive) detectado automaticamente nas pastas comuns.</p>`;
        } else {
            listEl.innerHTML = folders.map(f => {
                const targetFolder = f.savevault_folder || f.sync_folder || f.path || "";
                const safePath = targetFolder.replace(/\\/g, "\\\\");
                const provType = f.provider_type || f.provider || "";
                const identified = identifyCloudProvider(provType, targetFolder, f.name);
                const icon = identified === "onedrive" ? getOneDriveIcon(22) 
                           : identified === "google_drive" ? getGoogleDriveIcon(20) 
                           : "📁";
                return `
                <div class="content-card" style="padding:10px 14px;display:flex;align-items:center;justify-content:space-between;cursor:pointer;" onclick="window.selectCloudFolder('${provType || identified}', '${safePath}')">
                    <div style="display:flex;align-items:center;gap:12px;">
                        <span style="display:flex;align-items:center;justify-content:center;width:24px;height:24px;">${icon}</span>
                        <div>
                            <p style="font-weight:600;font-size:13px;" class="game-name">${f.name || "Nuvem"}</p>
                            <p class="game-subtext" style="font-size:11.5px;font-family:ui-monospace, monospace;">${targetFolder}</p>
                        </div>
                    </div>
                    <button type="button" class="btn-secondary" style="font-size:11px;padding:4px 10px;">Usar Esta</button>
                </div>
            `;
            }).join("");
        }
    }

    modal?.classList.remove("hidden");
};

window.selectCloudFolder = function(provider, path) {
    const customInput = document.getElementById("cloud-custom-path");
    if (customInput) customInput.value = path;
};

document.getElementById("btn-open-cloud-config")?.addEventListener("click", () => {
    window.openCloudModal();
});
document.getElementById("btn-close-cloud")?.addEventListener("click", () => {
    document.getElementById("modal-cloud-config")?.classList.add("hidden");
});
document.getElementById("btn-cancel-cloud")?.addEventListener("click", () => {
    document.getElementById("modal-cloud-config")?.classList.add("hidden");
});

document.getElementById("btn-save-cloud")?.addEventListener("click", async () => {
    const customPath = document.getElementById("cloud-custom-path")?.value?.trim() || "";
    const btn = document.getElementById("btn-save-cloud");
    if (btn) {
        btn.textContent = "Salvando...";
        btn.disabled = true;
    }

    try {
        let provider = "none";
        if (customPath) {
            if (customPath.toLowerCase().includes("onedrive")) {
                provider = "onedrive";
            } else if (customPath.toLowerCase().includes("google")) {
                provider = "google_drive";
            } else {
                provider = "local_folder";
            }
        }
        const enabled = Boolean(customPath);
        await invoke("update_cloud_config", {
            provider,
            localSyncPath: customPath || null,
            enabled,
        });
        showToast("Configuração da nuvem salva com sucesso!", "success");
        document.getElementById("modal-cloud-config")?.classList.add("hidden");
        await loadNuvem();
        await updateCloudBadge();
    } catch (e) {
        showToast(`Erro ao salvar nuvem: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "Salvar Provedor";
            btn.disabled = false;
        }
    }
});

// ── Aba: Configurações ────────────────────────────────────────────────────────

async function loadConfig() {
    try {
        const cfg = await invoke("get_config");

        const elBackupDir = document.getElementById("cfg-backup-dir");
        const elCloudSync = document.getElementById("cfg-cloud-sync-path");
        const elDbPath = document.getElementById("cfg-database-path");
        const elMaxVersions = document.getElementById("cfg-max-versions");
        const elReconcile = document.getElementById("cfg-reconcile-interval");
        const elDebounce = document.getElementById("cfg-debounce-ms");
        const elLogLevel = document.getElementById("cfg-log-level");
        const elAutostart = document.getElementById("cfg-autostart");

        if (elBackupDir) elBackupDir.value = cfg.backup_dir || "";
        if (elCloudSync) elCloudSync.value = cfg.cloud_sync_path || "";
        if (elDbPath) elDbPath.value = cfg.database_path || "";
        if (elMaxVersions) elMaxVersions.value = String(cfg.max_versions || 10);
        if (elReconcile) elReconcile.value = String(cfg.reconciliation_interval_secs || 30);
        if (elDebounce) elDebounce.value = String(cfg.debounce_ms || 2000);
        if (elLogLevel) elLogLevel.value = cfg.log_level || "info";
        if (elAutostart) elAutostart.checked = Boolean(cfg.autostart);
    } catch (e) {
        console.error("Erro ao carregar configurações:", e);
    }
}

// Botão Salvar Configurações
document.getElementById("btn-save-config")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-save-config");
    if (btn) {
        btn.textContent = "Salvando...";
        btn.disabled = true;
    }

    try {
        const backupDir = document.getElementById("cfg-backup-dir")?.value?.trim() || "";
        const cloudSyncPath = document.getElementById("cfg-cloud-sync-path")?.value?.trim() || null;
        const maxVersions = parseInt(document.getElementById("cfg-max-versions")?.value, 10) || 10;
        const reconciliationInterval = parseInt(document.getElementById("cfg-reconcile-interval")?.value, 10) || 30;
        const debounceMs = parseInt(document.getElementById("cfg-debounce-ms")?.value, 10) || 2000;
        const logLevel = document.getElementById("cfg-log-level")?.value || "info";
        const databasePath = document.getElementById("cfg-database-path")?.value || "";
        const autostart = Boolean(document.getElementById("cfg-autostart")?.checked);

        const payload = {
            max_versions: maxVersions,
            debounce_ms: debounceMs,
            reconciliation_interval_secs: reconciliationInterval,
            log_level: logLevel,
            backup_dir: backupDir,
            database_path: databasePath,
            autostart,
            cloud_sync_path: cloudSyncPath,
        };

        const result = await invoke("save_config", { newCfg: payload });
        showToast(result || "Configurações salvas com sucesso!", "success");
        await loadConfig();
        await loadNuvem();
        await updateCloudBadge();
    } catch (e) {
        showToast(`Erro ao salvar configurações: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "Salvar Alterações";
            btn.disabled = false;
        }
    }
});

// Botão Abrir Pasta de Backups no Explorer
document.getElementById("btn-open-backup-dir")?.addEventListener("click", () => {
    const path = document.getElementById("cfg-backup-dir")?.value;
    if (path) {
        invoke("open_path_in_explorer", { path }).catch(err => showToast(`Erro: ${err}`, "error"));
    }
});

// Botão Abrir Pasta de Nuvem no Explorer
document.getElementById("btn-open-cloud-sync-dir")?.addEventListener("click", () => {
    const path = document.getElementById("cfg-cloud-sync-path")?.value;
    if (path) {
        invoke("open_path_in_explorer", { path }).catch(err => showToast(`Erro: ${err}`, "error"));
    }
});

// Botão Abrir Pasta do Banco no Explorer
document.getElementById("btn-open-db-dir")?.addEventListener("click", () => {
    const path = document.getElementById("cfg-database-path")?.value;
    if (path) {
        invoke("open_path_in_explorer", { path }).catch(err => showToast(`Erro: ${err}`, "error"));
    }
});

// Botão Manutenção: Otimizar Banco SQLite
document.getElementById("btn-optimize-db")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-optimize-db");
    if (btn) {
        btn.textContent = "Otimizando...";
        btn.disabled = true;
    }
    try {
        const res = await invoke("optimize_database");
        showToast(`Banco otimizado! ${res.purged_sync_items} itens antigos limpos. Tamanho atual: ${res.db_size_formatted}`, "success");
    } catch (e) {
        showToast(`Falha na otimização: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "🧹 Otimizar Banco de Dados";
            btn.disabled = false;
        }
    }
});

// Botão Manutenção: Exportar Backups em ZIP
document.getElementById("btn-export-zip")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-export-zip");
    if (btn) {
        btn.textContent = "Gerando ZIP...";
        btn.disabled = true;
    }
    try {
        const res = await invoke("export_backup_zip");
        showToast(`Backup ZIP gerado com sucesso (${res.file_size_formatted}) em Downloads!`, "success");
    } catch (e) {
        showToast(`Falha na exportação: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "📦 Exportar Backups (ZIP)";
            btn.disabled = false;
        }
    }
});

// ── Botão Sincronizar Agora ──────────────────────────────────────────────────

document.getElementById("btn-reconcile")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-reconcile");
    if (btn) {
        btn.textContent = "Sincronizando...";
        btn.disabled = true;
    }
    try {
        const result = await invoke("trigger_reconcile");
        showToast(`Reconciliação concluída: ${result}`, "success");
        loadInicio();
    } catch (e) {
        showToast(`Erro na reconciliação: ${e}`, "error");
    } finally {
        if (btn) {
            btn.textContent = "Sincronizar Agora";
            btn.disabled = false;
        }
    }
});

// ── Inicialização ────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", () => {
    initTheme();
    showTab("nav-inicio");
    loadInicio();
    setInterval(() => {
        const activeTab = document.querySelector(".nav-item.active")?.id;
        if (activeTab === "nav-inicio") {
            loadInicio();
        } else if (activeTab === "nav-atividade") {
            loadAtividade();
        }
    }, 10000);
});
