(async () => {
  const statusText = document.getElementById("statusText");
  const closeButton = document.getElementById("closeButton");
  const minimizeButton = document.getElementById("minimizeButton");
  const launchButton = document.getElementById("launchButton");
  const loaderFill = document.getElementById("loaderFill");
  const profileAvatarButton = document.getElementById("profileAvatarButton");
  const profileAvatar = document.getElementById("profileAvatar");
  const avatarFileInput = document.getElementById("avatarFileInput");
  const profileUsername = document.getElementById("profileUsername");
  const profileExpiration = document.getElementById("profileExpiration");
  const profileExpirationButton = document.getElementById("profileExpirationButton");
  const profileExpirationText = document.getElementById("profileExpirationText");
  const socialLinks = Array.from(document.querySelectorAll(".brand-social-link"));
  const productButtons = Array.from(document.querySelectorAll("[data-product]"));
  const dragStrip = document.querySelector(".drag-strip");
  const PRODUCT_COMMANDS = Object.freeze({
    gs: "start_gamesense_setup",
    primo: "start_primo_setup",
    nl: "start_setup",
  });
  const LAUNCH_FADE_MS = 420;
  const FILL_ANIMATION_MS = 900;
  const MAX_AVATAR_SIZE = 100;
  const MILLISECONDS_PER_SECOND = 1_000;
  const MILLISECONDS_PER_MINUTE = 60_000;
  const DATETIME_LOCAL_VALUE_LENGTH = 16;
  let selectedProduct = "nl";
  let setupStarted = false;

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

  function selectProduct(product) {
    if (setupStarted) return;

    selectedProduct = product;
    document.body.dataset.product = product;
    document.body.dataset.statusLevel = "info";
    if (statusText) statusText.textContent = "";

    for (const button of productButtons) {
      button.classList.toggle("menu-item-active", button.dataset.product === product);
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

  function showProfileError(error) {
    document.body.dataset.statusLevel = "error";
    showRuntimeError(error);
  }

  function runtimeErrorContent(error) {
    const message = String(error).replace(/^Error:\s*/i, "");

    if (message.includes("код 32") || message.includes("занят запущенным процессом")) {
      return {
        title: "Файл занят",
        detail: "Закрой запущенный инжектор и нажми Launch ещё раз.",
      };
    }

    if (message.includes("код 5") || message.includes("PermissionDenied")) {
      return {
        title: "Нет доступа к файлу",
        detail: "Закрой программу, которая использует файл, и повтори запуск.",
      };
    }

    return {
      title: "Не удалось выполнить запуск",
      detail: message,
    };
  }

  function showRuntimeError(error) {
    if (!statusText) return;

    const content = runtimeErrorContent(error);
    const title = document.createElement("strong");
    const detail = document.createElement("span");
    title.className = "runtime-error-title";
    detail.className = "runtime-error-detail";
    title.textContent = content.title;
    detail.textContent = content.detail;
    statusText.replaceChildren(title, detail);
  }

  function timestampToInputValue(timestamp) {
    const date = new Date(timestamp * MILLISECONDS_PER_SECOND);
    const localDate = new Date(date.getTime() - date.getTimezoneOffset() * MILLISECONDS_PER_MINUTE);
    return localDate.toISOString().slice(0, DATETIME_LOCAL_VALUE_LENGTH);
  }

  function timestampToDisplayValue(timestamp) {
    const parts = new Intl.DateTimeFormat("ru-RU", {
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    })
      .formatToParts(new Date(timestamp * MILLISECONDS_PER_SECOND))
      .reduce((result, part) => {
        result[part.type] = part.value;
        return result;
      }, {});

    return `${parts.day}.${parts.month}.${parts.year} ${parts.hour}:${parts.minute}`;
  }

  function showUninitializedProfile() {
    profileAvatar.src = "./assets/images/nl.png";
    profileAvatar.classList.add("is-placeholder");
    profileUsername.value = "Profile";
    profileExpirationText.textContent = "After first launch";
    profileAvatarButton.disabled = true;
    profileUsername.disabled = true;
    profileExpiration.disabled = true;
    profileExpirationButton.disabled = true;
  }

  async function loadNeverloseProfile() {
    if (!window.__TAURI__?.core?.invoke) return;

    try {
      const profile = await window.__TAURI__.core.invoke("get_neverlose_profile");
      if (!profile) {
        showUninitializedProfile();
        return;
      }

      profileAvatar.src = profile.avatarDataUrl;
      profileAvatar.classList.remove("is-placeholder");
      profileUsername.value = profile.username;
      profileExpiration.value = timestampToInputValue(profile.expirationDate);
      profileExpirationText.textContent = timestampToDisplayValue(profile.expirationDate);
      profileAvatarButton.disabled = false;
      profileUsername.disabled = false;
      profileExpiration.disabled = false;
      profileExpirationButton.disabled = false;
    } catch (error) {
      showUninitializedProfile();
    }
  }

  async function imageFileToClampedPng(file) {
    const imageUrl = URL.createObjectURL(file);
    const image = new Image();

    try {
      await new Promise((resolve, reject) => {
        image.onload = resolve;
        image.onerror = () => reject(new Error("Не удалось открыть выбранное изображение."));
        image.src = imageUrl;
      });

      const scale = Math.min(
        1,
        MAX_AVATAR_SIZE / image.naturalWidth,
        MAX_AVATAR_SIZE / image.naturalHeight,
      );
      const width = Math.max(1, Math.round(image.naturalWidth * scale));
      const height = Math.max(1, Math.round(image.naturalHeight * scale));
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d");
      context.drawImage(image, 0, 0, width, height);
      return canvas.toDataURL("image/png");
    } finally {
      URL.revokeObjectURL(imageUrl);
    }
  }

  profileAvatarButton?.addEventListener("click", () => avatarFileInput?.click());
  profileExpirationButton?.addEventListener("click", () => profileExpiration?.showPicker());

  socialLinks.forEach((link) => {
    link.addEventListener("click", async (event) => {
      event.preventDefault();
      try {
        await window.__TAURI__.opener.openUrl(link.href);
      } catch (error) {
        showProfileError(error);
      }
    });
  });

  avatarFileInput?.addEventListener("change", async () => {
    const file = avatarFileInput.files?.[0];
    if (!file || !window.__TAURI__?.core?.invoke) return;

    try {
      const avatarData = await imageFileToClampedPng(file);
      await window.__TAURI__.core.invoke("save_neverlose_avatar", { avatarData });
      profileAvatar.src = avatarData;
      avatarFileInput.value = "";
    } catch (error) {
      showProfileError(error);
    }
  });

  profileUsername?.addEventListener("change", async () => {
    try {
      await window.__TAURI__.core.invoke("save_neverlose_username", {
        username: profileUsername.value,
      });
    } catch (error) {
      showProfileError(error);
    }
  });

  profileUsername?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") profileUsername.blur();
  });

  profileExpiration?.addEventListener("change", async () => {
    const expirationDate = Math.floor(
      new Date(profileExpiration.value).getTime() / MILLISECONDS_PER_SECOND,
    );
    if (!Number.isFinite(expirationDate)) {
      showProfileError("Укажи дату и время полностью.");
      return;
    }
    try {
      await window.__TAURI__.core.invoke("save_neverlose_expiration", { expirationDate });
      profileExpirationText.textContent = timestampToDisplayValue(expirationDate);
    } catch (error) {
      showProfileError(error);
    }
  });

  function applyEvent(payload) {
    if (!payload || typeof payload !== "object") return;

    document.body.dataset.statusLevel = payload.level || "info";

    if (typeof payload.percent === "number" && payload.level !== "error") {
      setLoaderProgress(payload.percent);
    }

    if (payload.level === "error") {
      setupStarted = false;
      resetLoaderVisuals();
      showRuntimeError(payload.message || "");
      setLaunchState("Launch", false);
    } else if (payload.stage === "done") {
      setupStarted = false;
      setLoaderProgress(100);
      if (selectedProduct === "nl") {
        loadNeverloseProfile();
      }
      window.setTimeout(() => {
        document.body.dataset.loading = "false";
        document.body.dataset.done = "true";
        if (statusText) statusText.textContent = "";
      }, FILL_ANIMATION_MS);
    }
  }

  async function startSequence() {
    if (setupStarted || !window.__TAURI__?.core?.invoke) return;

    setupStarted = true;
    setLoaderProgress(0);
    document.body.dataset.statusLevel = "info";
    document.body.dataset.loading = "true";
    document.body.dataset.done = "false";

    if (statusText) statusText.textContent = "";
    setLaunchState("Launch", true);
    if (launchButton) {
      launchButton.classList.add("is-fading");
      window.setTimeout(() => {
        if (setupStarted && launchButton) launchButton.hidden = true;
      }, LAUNCH_FADE_MS);
    }

    try {
      await window.__TAURI__.core.invoke(PRODUCT_COMMANDS[selectedProduct]);
    } catch (error) {
      setupStarted = false;
      document.body.dataset.statusLevel = "error";
      resetLoaderVisuals();
      showRuntimeError(error);
      setLaunchState("Launch", false);
    }
  }

  launchButton?.addEventListener("click", startSequence);
  productButtons.forEach((button) => {
    button.addEventListener("click", () => selectProduct(button.dataset.product || "nl"));
  });
  selectProduct("nl");

  if (window.__TAURI__?.event?.listen) {
    const profileRequest = loadNeverloseProfile();
    await window.__TAURI__.event.listen("setup://status", (event) => {
      applyEvent(event.payload);
    });
    await profileRequest;
  } else if (statusText) {
    statusText.textContent = "Tauri API is not loaded.";
    if (launchButton) launchButton.disabled = true;
  }
})();
