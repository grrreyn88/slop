(async () => {
  const statusText = document.getElementById("statusText");
  const closeButton = document.getElementById("closeButton");
  const minimizeButton = document.getElementById("minimizeButton");
  const launchButton = document.getElementById("launchButton");
  const loaderFill = document.getElementById("loaderFill");
  const doneMessage = document.getElementById("doneMessage");
  const telegramLink = document.getElementById("telegramLink");
  const gameIconImage = document.getElementById("gameIconImage");
  const gameIconFallback = document.getElementById("gameIconFallback");
  const dragStrip = document.querySelector(".drag-strip");
  const launchFadeMs = 420;
  const fillAnimationMs = 900;
  let setupStarted = false;
  let closeTimer = null;

  function setLaunchState(text, disabled) {
    if (!launchButton) return;

    launchButton.disabled = disabled;
    const label = launchButton.querySelector("span");
    if (label) {
      label.textContent = text;
    } else {
      launchButton.textContent = text;
    }
  }

  function setLoaderProgress(value) {
    const safeValue = Math.max(0, Math.min(100, value));
    loaderFill?.style.setProperty("--loader-progress", `${safeValue}%`);
  }

  function resetLoaderVisuals() {
    document.body.dataset.loading = "false";
    document.body.dataset.done = "false";
    setLoaderProgress(0);
    if (launchButton) {
      launchButton.classList.remove("is-fading");
      launchButton.hidden = false;
    }
  }

  dragStrip?.addEventListener("mousedown", async (e) => {
    if (e.button === 0 && window.__TAURI__?.core?.invoke) {
      await window.__TAURI__.core.invoke("drag_app");
    }
  });

  closeButton?.addEventListener("click", async () => {
    if (window.__TAURI__?.core?.invoke) {
      await window.__TAURI__.core.invoke("exit_app").catch(() => window.close());
    } else {
      window.close();
    }
  });

  minimizeButton?.addEventListener("click", async () => {
    if (window.__TAURI__?.core?.invoke) {
      await window.__TAURI__.core.invoke("minimize_app");
    }
  });

  telegramLink?.addEventListener("click", async (event) => {
    event.preventDefault();
    const url = telegramLink.href;

    if (window.__TAURI__?.core?.invoke) {
      await window.__TAURI__.core.invoke("open_telegram_link", { url }).catch(() => {
        window.location.href = url;
      });
    } else {
      window.location.href = url;
    }
  });

  async function loadGameIcon() {
    if (!window.__TAURI__?.core?.invoke || !gameIconImage) return;

    try {
      const iconDataUrl = await window.__TAURI__.core.invoke("get_csgo_icon");
      if (typeof iconDataUrl === "string" && iconDataUrl.startsWith("data:image/")) {
        gameIconImage.src = iconDataUrl;
        gameIconImage.hidden = false;
        if (gameIconFallback) gameIconFallback.hidden = true;
      }
    } catch {
    }
  }

  function setProgress(value) {
    const safeValue = Math.max(0, Math.min(100, value));
    setLoaderProgress(safeValue);
  }

  function applyEvent(payload) {
    if (!payload || typeof payload !== "object") return;

    document.body.dataset.statusLevel = payload.level || "info";

    if (typeof payload.percent === "number" && payload.level !== "error") {
      setProgress(payload.percent);
    }

    if (payload.level === "error") {
      setupStarted = false;
      if (closeTimer) {
        window.clearTimeout(closeTimer);
        closeTimer = null;
      }
      resetLoaderVisuals();
      if (statusText) statusText.textContent = payload.message || "";
      setLaunchState("Launch", false);
    } else if (payload.stage === "done") {
      setupStarted = false;
      setLoaderProgress(100);
      window.setTimeout(() => {
        document.body.dataset.loading = "false";
        document.body.dataset.done = "true";
        if (statusText) statusText.textContent = "";
        closeTimer =
          closeTimer ||
          window.setTimeout(async () => {
            if (window.__TAURI__?.core?.invoke) {
              await window.__TAURI__.core.invoke("close_loader").catch(() => window.close());
            } else {
              window.close();
            }
          }, 3000);
      }, fillAnimationMs);
    }
  }

  async function startSequence() {
    if (setupStarted || !window.__TAURI__?.core?.invoke) return;

    setupStarted = true;
    if (closeTimer) {
      window.clearTimeout(closeTimer);
      closeTimer = null;
    }
    setProgress(0);
    document.body.dataset.statusLevel = "info";
    document.body.dataset.loading = "true";
    document.body.dataset.done = "false";

    if (statusText) statusText.textContent = "";
    setLaunchState("Launch", true);
    if (launchButton) {
      launchButton.classList.add("is-fading");
      window.setTimeout(() => {
        if (setupStarted && launchButton) launchButton.hidden = true;
      }, launchFadeMs);
    }

    try {
      await window.__TAURI__.core.invoke("start_setup");
    } catch (error) {
      setupStarted = false;
      document.body.dataset.statusLevel = "error";
      resetLoaderVisuals();
      if (statusText) statusText.textContent = "Error: " + String(error);
      setLaunchState("Launch", false);
    }
  }

  launchButton?.addEventListener("click", startSequence);
  await loadGameIcon();
  window.setTimeout(loadGameIcon, 1200);

  if (window.__TAURI__?.event?.listen) {
    await window.__TAURI__.event.listen("setup://status", (event) => {
      applyEvent(event.payload);
    });
  } else if (statusText) {
    statusText.textContent = "Tauri API is not loaded.";
    if (launchButton) launchButton.disabled = true;
  }
})();
