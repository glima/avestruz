// Which page this is: the plain one, or the one with a scene to walk about in.
//
// The page is whole without any of this: every word of it is in the HTML.
// The scene is drawn over the top of the page's first screen when the
// machine looks able to, or when it is asked for; the choice is kept.

const KEY = "avestruz-site-mode";
const MODES = ["auto", "hifi", "lofi"];

const root = document.documentElement;
const stage = document.getElementById("stage");
const note = document.getElementById("stage-note");
const buttons = [...document.querySelectorAll("[data-mode]")];

/// Whether a scene would likely run well here, as far as a page can tell.
function capable() {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
  if (navigator.connection?.saveData) return false;
  if (Math.min(innerWidth, innerHeight) < 560 || innerWidth < 900) return false;
  if ((navigator.deviceMemory ?? 8) < 4 || (navigator.hardwareConcurrency ?? 8) < 4) return false;
  const gl = document.createElement("canvas").getContext("webgl2");
  if (!gl) return false;
  // A renderer that draws on the processor would run it as a slide show.
  const info = gl.getExtension("WEBGL_debug_renderer_info");
  const renderer = info ? String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL)) : "";
  return !/swiftshader|llvmpipe|softpipe|software/i.test(renderer);
}

let world = null;
let starting = null;

async function apply(mode) {
  for (const button of buttons) button.setAttribute("aria-pressed", String(button.dataset.mode === mode));
  const scene = mode === "hifi" || (mode === "auto" && capable());
  if (!scene) {
    root.classList.remove("hifi");
    world?.pause();
    return;
  }
  root.classList.add("hifi", "loading");
  note.hidden = true;
  try {
    starting ??= import("./world.js").then((module) => module.start(stage));
    world = await starting;
    world.resume();
  } catch (error) {
    console.warn("the scene could not start", error);
    starting = null;
    root.classList.remove("hifi");
    note.hidden = false;
  } finally {
    root.classList.remove("loading");
  }
}

function stored() {
  try {
    const mode = localStorage.getItem(KEY);
    return MODES.includes(mode) ? mode : "auto";
  } catch {
    return "auto";
  }
}

for (const button of buttons) {
  button.addEventListener("click", () => {
    try {
      localStorage.setItem(KEY, button.dataset.mode);
    } catch {
      // Nowhere to keep it: the choice lasts for this visit.
    }
    apply(button.dataset.mode);
  });
}
document.querySelector(".modes").hidden = false;

// `?mode=hifi` shows the scene whatever was chosen, without keeping it.
const asked = new URLSearchParams(location.search).get("mode");
apply(MODES.includes(asked) ? asked : stored());
