// The plain at dusk: a relay outpost to walk about in, where each thing
// standing on it is a piece of Nostr.
//
// The props are files made by `site/craft` (SolidCraft shapes, TextureCraft
// surfaces); what is here sets them out, lights them, keeps them moving and
// lets the visitor go up to each one. The words shown beside a prop are
// the page's own: the `<article data-subject>` elements of its Nostr
// section, so they read the same with the scene and without.

import * as THREE from "three";
import { MeshoptDecoder } from "three/addons/libs/meshopt_decoder.module.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";

const ASSETS = new URL("../assets/", import.meta.url);

// Where each prop stands (metres; x to the right, z towards the visitor)
// and how far it is turned about its own upright, in radians.
const PLACES = {
  ostrich: { at: [0.3, 4.6], turn: -0.3 },
  bunker: { at: [-6.3, 0.0], turn: 0.3 },
  mailbox: { at: [-4.8, 4.6], turn: -0.6 },
  book: { at: [-2.4, 7.0], turn: 0.2 },
  blossom: { at: [3.2, 4.9], turn: 0 },
  telescope: { at: [6.6, 2.6], turn: 0.5 },
  rod: { at: [9.4, -1.6], turn: 0.4 },
  billboard: { at: [4.2, -6.4], turn: -0.25 },
  tower: { at: [7.0, -13.5], turn: 0.4 },
};
// The poles stand in a line, and the last is wired to the tower.
const POLES = [-22, -13, -4, 5].map((x) => [x, -9.5]);
const TREES = [
  { at: [-17, -21], size: 1.35, turn: 0.4 },
  { at: [17, -27], size: 1.7, turn: 2.1 },
  { at: [25, -9], size: 1.1, turn: 4.0 },
];

// What can be gone up to: which props stand for it, where the camera goes
// to look (from, and at, both from the first prop's foot) and how wide the
// ring on the ground under it is.
const SUBJECTS = {
  nostr: { props: ["ostrich"], from: [1.2, 2.2, 6.8], look: [1.2, 1.9, 0], ring: 1.5 },
  relays: { props: ["tower"], from: [-5, 8, 12], look: [1.5, 5.2, 0], ring: 1.6 },
  notes: { props: ["pole2", "pole3"], from: [4, 3.4, 11], look: [0.5, 4.7, 0], ring: 0.8 },
  keys: { props: ["bunker"], from: [1.5, 1.9, 6.5], look: [0.6, 0.8, 0], ring: 2.3 },
  zaps: { props: ["rod", "cloud"], from: [-4, 3.6, 14], look: [1.6, 4.1, 0], ring: 1.4 },
  media: { props: ["blossom"], from: [1.2, 1.5, 4.2], look: [0.5, 1.0, 0], ring: 0.7 },
  writing: { props: ["mailbox"], from: [2.0, 1.6, 3.8], look: [0.5, 1.1, 0], ring: 0.7 },
  panel: { props: ["billboard"], from: [1.5, 2.6, 8.5], look: [0.9, 2.5, 0], ring: 2.0 },
  search: { props: ["telescope"], from: [-1.5, 1.8, 4.6], look: [0.5, 1.1, 0], ring: 0.9 },
  bookmarks: { props: ["book"], from: [1.0, 1.5, 3.2], look: [0.3, 0.5, 0], ring: 1.0 },
};

const HOME = { from: new THREE.Vector3(1.0, 3.2, 17.5), look: new THREE.Vector3(1.4, 3.0, 0) };
const PURPLE = new THREE.Color(0xa970ff);
const ZAP = new THREE.Color(0xffb347);
// Where a note is that is not on its way: drawn as nothing.
const NOWHERE = new THREE.Matrix4().makeScale(0, 0, 0);

/// The same numbers every visit, so the grass and the rocks are where they were.
function seeded(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── The sky ───────────────────────────────────────────────────────────

const SUN = new THREE.Vector3(-0.72, 0.02, -0.69).normalize();

function sky() {
  const dome = new THREE.Mesh(
    new THREE.SphereGeometry(400, 48, 24),
    new THREE.ShaderMaterial({
      side: THREE.BackSide,
      depthWrite: false,
      fog: false,
      uniforms: { sun: { value: SUN } },
      vertexShader: `
        varying vec3 direction;
        void main() {
          direction = normalize(position);
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }`,
      fragmentShader: `
        uniform vec3 sun;
        varying vec3 direction;
        void main() {
          vec3 d = normalize(direction);
          float up = clamp(d.y, 0.0, 1.0);
          vec3 zenith = vec3(0.012, 0.014, 0.05);
          vec3 high = vec3(0.045, 0.03, 0.12);
          vec3 low = vec3(0.36, 0.13, 0.22);
          vec3 color = mix(low, high, smoothstep(0.0, 0.26, up));
          color = mix(color, zenith, smoothstep(0.12, 0.6, up));
          // Where the sun went down, the horizon still burns.
          float toward = max(dot(normalize(vec3(d.x, 0.0, d.z)), normalize(vec3(sun.x, 0.0, sun.z))), 0.0);
          float band = 1.0 - smoothstep(0.0, 0.3, up);
          color += vec3(1.5, 0.52, 0.12) * pow(toward, 5.0) * band;
          color += vec3(0.55, 0.16, 0.2) * pow(toward, 1.5) * band * 0.6;
          // Under the horizon: the haze the plain fades into.
          color = mix(color, vec3(0.085, 0.05, 0.11), smoothstep(0.0, -0.06, d.y));
          gl_FragColor = vec4(color, 1.0);
        }`,
    }),
  );
  return dome;
}

function stars(random) {
  const count = 1400;
  const positions = new Float32Array(count * 3);
  const sparks = new Float32Array(count * 2);
  for (let i = 0; i < count; i++) {
    const turn = random() * Math.PI * 2;
    const height = 0.06 + random() * 0.94;
    const flat = Math.sqrt(1 - height * height);
    positions.set([Math.cos(turn) * flat * 380, height * 380, Math.sin(turn) * flat * 380], i * 3);
    sparks.set([0.6 + random() ** 3 * 2.6, random() * 6.283], i * 2);
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("spark", new THREE.BufferAttribute(sparks, 2));
  const material = new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    fog: false,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 }, scale: { value: 1 } },
    vertexShader: `
      attribute vec2 spark;
      uniform float time;
      uniform float scale;
      varying float glow;
      void main() {
        // Dimmer towards the horizon, where the dusk is, and never still.
        float height = normalize(position).y;
        glow = spark.x * smoothstep(0.05, 0.45, height) * (0.7 + 0.3 * sin(time * (0.6 + spark.x) + spark.y));
        gl_PointSize = (1.2 + spark.x) * scale;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }`,
    fragmentShader: `
      varying float glow;
      void main() {
        float fall = 1.0 - smoothstep(0.0, 0.5, length(gl_PointCoord - 0.5));
        gl_FragColor = vec4(vec3(0.85, 0.88, 1.0) * glow * fall, 1.0);
      }`,
  });
  return new THREE.Points(geometry, material);
}

/// The far edge of the plain: a ragged dark line under the glow.
function hills(random) {
  const geometry = new THREE.CylinderGeometry(180, 180, 1, 160, 1, true);
  const position = geometry.attributes.position;
  const phases = [random() * 6, random() * 6, random() * 6];
  for (let i = 0; i < position.count; i++) {
    if (position.getY(i) < 0) {
      position.setY(i, -2);
      continue;
    }
    const turn = Math.atan2(position.getZ(i), position.getX(i));
    const height = 5 + 3.2 * Math.sin(turn * 3 + phases[0]) + 2.1 * Math.sin(turn * 7 + phases[1]) + 1.2 * Math.sin(turn * 17 + phases[2]);
    position.setY(i, Math.max(1.5, height));
  }
  return new THREE.Mesh(geometry, new THREE.MeshBasicMaterial({ color: 0x0c0914, side: THREE.BackSide, fog: false }));
}

// ── The ground ────────────────────────────────────────────────────────

async function ground(textures) {
  const [color, normal] = await Promise.all([
    textures.loadAsync(new URL("tex/sand_color.webp", ASSETS).href),
    textures.loadAsync(new URL("tex/sand_normal.webp", ASSETS).href),
  ]);
  for (const texture of [color, normal]) {
    texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
    texture.repeat.set(70, 70);
    texture.anisotropy = 8;
  }
  color.colorSpace = THREE.SRGBColorSpace;
  const plain = new THREE.Mesh(
    new THREE.CircleGeometry(190, 96).rotateX(-Math.PI / 2),
    new THREE.MeshStandardMaterial({ map: color, normalMap: normal, normalScale: new THREE.Vector2(1.6, 1.6), roughness: 1, color: 0xb9a48a }),
  );
  plain.receiveShadow = true;
  return plain;
}

/// Dry grass, in tufts, thicker away from where things stand.
function grass(random, clear) {
  const count = 3200;
  const blade = new THREE.ConeGeometry(0.03, 0.6, 3).translate(0, 0.3, 0);
  const tufts = new THREE.InstancedMesh(blade, new THREE.MeshStandardMaterial({ roughness: 1 }), count);
  const matrix = new THREE.Matrix4();
  const lean = new THREE.Euler();
  const turn = new THREE.Quaternion();
  const size = new THREE.Vector3();
  const at = new THREE.Vector3();
  const tint = new THREE.Color();
  let placed = 0;
  while (placed < count) {
    const angle = random() * Math.PI * 2;
    const reach = 2 + random() ** 0.7 * 44;
    const tuft = [Math.cos(angle) * reach, Math.sin(angle) * reach - 6];
    if (clear.some(([x, z, r]) => Math.hypot(tuft[0] - x, tuft[1] - z) < r)) continue;
    const blades = 5 + Math.floor(random() * 9);
    tint.setHSL(0.11 + random() * 0.05, 0.42, 0.26 + random() * 0.14);
    for (let i = 0; i < blades && placed < count; i++, placed++) {
      at.set(tuft[0] + (random() - 0.5) * 0.5, 0, tuft[1] + (random() - 0.5) * 0.5);
      lean.set((random() - 0.5) * 0.7, random() * 6.283, (random() - 0.5) * 0.7);
      const tall = 0.5 + random() * 1.1;
      size.set(1, tall, 1);
      tufts.setMatrixAt(placed, matrix.compose(at, turn.setFromEuler(lean), size));
      tufts.setColorAt(placed, tint);
    }
  }
  tufts.receiveShadow = true;
  return tufts;
}

// ── What the screen on its posts shows ────────────────────────────────

/// A drawing of the popup: a feed, with the note the keyboard is at
/// moving down it. A drawing, not a picture of the real thing.
function screen() {
  const canvas = document.createElement("canvas");
  canvas.width = 750;
  canvas.height = 450;
  const pen = canvas.getContext("2d");
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 4;
  const random = seeded(7);
  const rows = Array.from({ length: 9 }, () => ({
    hue: Math.floor(random() * 360),
    name: 50 + random() * 70,
    lines: [0.55 + random() * 0.4, 0.3 + random() * 0.6, random() < 0.5 ? 0.2 + random() * 0.5 : 0],
    picture: random() < 0.3,
  }));
  const heights = rows.map((row) => 74 + (row.lines[2] ? 16 : 0) + (row.picture ? 84 : 0));
  let marked = -1;

  function bar(x, y, wide, tall, radius, fill) {
    pen.fillStyle = fill;
    pen.beginPath();
    pen.roundRect(x, y, wide, tall, radius);
    pen.fill();
  }

  function draw(step) {
    if (step === marked) return;
    marked = step;
    const at = step % rows.length;
    pen.fillStyle = "#12121a";
    pen.fillRect(0, 0, canvas.width, canvas.height);
    // The panel along the top, with the applet's button lit.
    bar(0, 0, canvas.width, 34, 0, "#1c1c26");
    for (let i = 0; i < 5; i++) bar(canvas.width - 40 - i * 30, 9, 16, 16, 4, i === 2 ? "#a970ff" : "#3a3a4a");
    bar(14, 11, 90, 12, 6, "#3a3a4a");
    // The popup under it.
    const left = 250;
    const wide = 380;
    bar(left, 44, wide, canvas.height - 54, 14, "#1f1f2b");
    bar(left + 16, 58, 86, 22, 11, "#a970ff");
    bar(left + 110, 58, 86, 22, 11, "#2c2c3b");
    for (let i = 0; i < 3; i++) bar(left + wide - 36 - i * 30, 60, 18, 18, 9, "#2c2c3b");
    pen.save();
    pen.beginPath();
    pen.roundRect(left, 92, wide, canvas.height - 102, 0);
    pen.clip();
    // The marked note is kept in view, as `j` and `k` keep it.
    let top = 96;
    for (let i = 0; i < at; i++) top -= heights[i] + 8;
    top = Math.min(96, top + 120);
    for (let i = 0; i < rows.length; i++) {
      const row = rows[i];
      bar(left + 10, top, wide - 20, heights[i], 10, "#282836");
      if (i === at) {
        pen.strokeStyle = "#a970ff";
        pen.lineWidth = 3;
        pen.beginPath();
        pen.roundRect(left + 10, top, wide - 20, heights[i], 10);
        pen.stroke();
      }
      pen.fillStyle = `hsl(${row.hue} 55% 60%)`;
      pen.beginPath();
      pen.arc(left + 36, top + 26, 15, 0, Math.PI * 2);
      pen.fill();
      bar(left + 60, top + 13, row.name, 10, 5, "#c9c6d8");
      let y = top + 32;
      for (const line of row.lines) {
        if (!line) continue;
        bar(left + 60, y, (wide - 90) * line, 8, 4, "#6f6c82");
        y += 16;
      }
      if (row.picture) {
        bar(left + 60, y, wide - 90, 76, 8, `hsl(${(row.hue + 40) % 360} 35% 38%)`);
        y += 84;
      }
      for (let k = 0; k < 4; k++) bar(left + 60 + k * 64, y + 2, 26, 8, 4, k === 3 && i % 3 === 0 ? "#ffb347" : "#45435a");
      top += heights[i] + 8;
    }
    pen.restore();
    texture.needsUpdate = true;
  }

  draw(0);
  const face = new THREE.Mesh(new THREE.PlaneGeometry(3.0, 1.8), new THREE.MeshBasicMaterial({ map: texture, color: new THREE.Color(1.25, 1.25, 1.25) }));
  face.position.set(0, 2.7, 0.08);
  return { face, draw };
}

// ── The scene ─────────────────────────────────────────────────────────

export async function start(stage) {
  const canvas = document.createElement("canvas");
  canvas.setAttribute("aria-hidden", "true");
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: "high-performance" });
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.15;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFShadowMap;

  const random = seeded(1984);
  const scene = new THREE.Scene();
  scene.fog = new THREE.FogExp2(0x1d1226, 0.0125);
  const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 900);

  const dome = sky();
  const night = stars(random);
  scene.add(dome, night, hills(random));
  // What shiny things reflect is the sky itself.
  const pmrem = new THREE.PMREMGenerator(renderer);
  const above = new THREE.Scene();
  above.add(dome.clone());
  scene.environment = pmrem.fromScene(above, 0, 1, 500).texture;
  scene.environmentIntensity = 0.9;
  pmrem.dispose();

  const moon = new THREE.Mesh(new THREE.CircleGeometry(5, 40), new THREE.MeshBasicMaterial({ color: new THREE.Color(1.7, 1.65, 1.5), fog: false }));
  moon.position.set(-30, 88, -300);
  moon.lookAt(0, 0, 0);
  scene.add(moon);

  // The last of the sun, low and from behind; the moon's cool light from
  // the front; and the sky's own, from everywhere.
  const sun = new THREE.DirectionalLight(0xff9d5c, 3.4);
  sun.position.set(-20, 9.5, -17);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.04;
  Object.assign(sun.shadow.camera, { left: -24, right: 24, top: 24, bottom: -24, near: 1, far: 80 });
  const moonlight = new THREE.DirectionalLight(0x8f9cff, 0.85);
  moonlight.position.set(9, 12, 14);
  scene.add(sun, moonlight, new THREE.HemisphereLight(0x6e6aa8, 0x3a2620, 1.15));

  // ── The props ──
  const loader = new GLTFLoader().setMeshoptDecoder(MeshoptDecoder);
  const names = ["ostrich", "ostrich_head", "tower", "pole", "bunker", "rod", "bolt", "cloud", "blossom", "mailbox", "billboard", "telescope", "book", "acacia", "rock"];
  const [plain, ...loaded] = await Promise.all([
    ground(new THREE.TextureLoader()),
    ...names.map((name) => loader.loadAsync(new URL(`models/${name}.glb`, ASSETS).href)),
  ]);
  const models = Object.fromEntries(names.map((name, i) => [name, loaded[i].scene]));
  const shining = {};
  for (const [name, model] of Object.entries(models)) {
    model.traverse((part) => {
      if (!part.isMesh) return;
      part.castShadow = part.receiveShadow = true;
      const look = part.material.name;
      if (look === "lamp" || look === "bolt" || look === "cloud") {
        part.castShadow = look === "cloud";
        (shining[`${name}.${look}`] ??= []).push(part.material);
      }
    });
  }
  scene.add(plain);

  const props = {};
  function place(name, model, [x, z], turn = 0, size = 1) {
    model.position.set(x, 0, z);
    model.rotation.y = turn;
    model.scale.setScalar(size);
    scene.add(model);
    props[name] = model;
    return model;
  }
  for (const [name, { at, turn }] of Object.entries(PLACES)) {
    const model = name === "ostrich" ? new THREE.Group().add(models.ostrich) : models[name];
    place(name, model, at, turn);
  }
  POLES.forEach((at, i) => place(`pole${i}`, i ? models.pole.clone() : models.pole, at));
  TREES.forEach(({ at, size, turn }, i) => place(`tree${i}`, i ? models.acacia.clone() : models.acacia, at, turn, size));
  for (let i = 0; i < 16; i++) {
    const angle = random() * Math.PI * 2;
    const reach = 9 + random() * 26;
    const rock = place(`rock${i}`, i ? models.rock.clone() : models.rock, [Math.cos(angle) * reach, Math.sin(angle) * reach - 4], random() * 6.283);
    rock.scale.set(0.5 + random() * 1.3, 0.4 + random() * 0.9, 0.5 + random() * 1.3);
  }
  const clear = [...Object.values(PLACES).map(({ at }) => [at[0], at[1], 2.2]), [0, 12, 5]];
  scene.add(grass(random, clear));

  // The bird's head turns on its neck's end.
  const head = models.ostrich_head;
  head.position.set(0.69, 2.62, 0);
  props.ostrich.add(head);

  // The screen, and the lamps over it.
  const show = screen();
  props.billboard.add(show.face);
  for (const material of shining["billboard.lamp"] ?? []) {
    material.emissive = new THREE.Color(0xffd9a0);
    material.emissiveIntensity = 3;
  }
  const glow = new THREE.PointLight(0xa58cff, 14, 9, 2);
  glow.position.set(0, 2.6, 1.6);
  props.billboard.add(glow);

  // The relay's lamp, and the rings that go out from it.
  const beacon = shining["tower.lamp"] ?? [];
  for (const material of beacon) material.emissive = new THREE.Color(0xff3b2e);
  const beaconLight = new THREE.PointLight(0xff4a3a, 0, 14, 2);
  beaconLight.position.set(0, 9.9, 0);
  props.tower.add(beaconLight);
  const rings = Array.from({ length: 3 }, () => {
    const ring = new THREE.Mesh(
      new THREE.RingGeometry(0.965, 1, 72).rotateX(-Math.PI / 2),
      new THREE.MeshBasicMaterial({ color: PURPLE.clone().multiplyScalar(1.3), transparent: true, depthWrite: false, side: THREE.DoubleSide, blending: THREE.AdditiveBlending, fog: false }),
    );
    ring.position.set(0, 9.2, 0);
    props.tower.add(ring);
    return ring;
  });

  // The lines between the poles, and from the last pole up to the relay.
  const lines = [];
  const wire = new THREE.MeshStandardMaterial({ color: 0x0d0b12, roughness: 0.6 });
  function hang(from, to, sag) {
    const points = Array.from({ length: 13 }, (_, i) => {
      const t = i / 12;
      return new THREE.Vector3().lerpVectors(from, to, t).setY(THREE.MathUtils.lerp(from.y, to.y, t) - sag * 4 * t * (1 - t));
    });
    const curve = new THREE.CatmullRomCurve3(points);
    scene.add(new THREE.Mesh(new THREE.TubeGeometry(curve, 28, 0.014, 5), wire));
    return curve;
  }
  const cups = [-0.7, -0.3, 0.3, 0.7];
  const top = (pole, cup) => new THREE.Vector3(pole[0], 5.235, pole[1] + cup);
  for (const cup of cups) {
    const line = [];
    // The line comes in from beyond the first pole, out of sight.
    line.push(hang(top([POLES[0][0] - 9, POLES[0][1]], cup), top(POLES[0], cup), 0.55));
    for (let i = 0; i + 1 < POLES.length; i++) line.push(hang(top(POLES[i], cup), top(POLES[i + 1], cup), 0.55));
    const deck = new THREE.Vector3(PLACES.tower.at[0] + cup * 0.3, 8.25, PLACES.tower.at[1] + 0.2);
    line.push(hang(top(POLES.at(-1), cup), deck, 0.35));
    lines.push(line);
  }

  // Notes on their way: along a line, pole to pole, to the relay or from it.
  const NOTES = 16;
  const paper = new THREE.InstancedMesh(new THREE.PlaneGeometry(0.26, 0.18), new THREE.MeshBasicMaterial({ fog: false }), NOTES);
  paper.frustumCulled = false;
  const travelling = Array.from({ length: NOTES }, () => ({ wait: random() * 9 }));
  const tints = [new THREE.Color(1.25, 1.15, 0.95), new THREE.Color(1.25, 1.15, 0.95), PURPLE.clone().multiplyScalar(1.5), ZAP.clone().multiplyScalar(1.4)];
  function send(note, i) {
    note.line = lines[Math.floor(random() * lines.length)];
    note.back = random() < 0.4;
    note.span = note.back ? note.line.length - 1 : 0;
    note.t = 0;
    note.speed = 0.13 + random() * 0.12;
    note.wait = 0;
    paper.setColorAt(i, tints[Math.floor(random() * tints.length)]);
    paper.instanceColor.needsUpdate = true;
  }
  scene.add(paper);

  // Lightning: the cloud over the rod, the bolt between them, its flash.
  const cloud = place("cloud", models.cloud, PLACES.rod.at);
  cloud.position.y = 7.3;
  const bolt = models.bolt;
  bolt.position.set(PLACES.rod.at[0], 3.9, PLACES.rod.at[1]);
  bolt.scale.setScalar(1.45);
  bolt.visible = false;
  scene.add(bolt);
  for (const material of shining["bolt.bolt"] ?? []) {
    material.emissive = ZAP.clone();
    material.emissiveIntensity = 9;
  }
  const stormy = shining["cloud.cloud"] ?? [];
  for (const material of stormy) material.emissive = new THREE.Color(0x9fb0ff);
  const flash = new THREE.PointLight(0xcfd8ff, 0, 60, 1.6);
  flash.position.set(PLACES.rod.at[0], 5.6, PLACES.rod.at[1] + 1);
  scene.add(flash);
  const sats = (() => {
    const label = document.createElement("canvas");
    label.width = 256;
    label.height = 96;
    const texture = new THREE.CanvasTexture(label);
    texture.colorSpace = THREE.SRGBColorSpace;
    const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: texture, transparent: true, depthWrite: false, fog: false }));
    sprite.scale.set(2.2, 0.82, 1);
    sprite.visible = false;
    scene.add(sprite);
    return {
      sprite,
      say(amount) {
        const pen = label.getContext("2d");
        pen.clearRect(0, 0, 256, 96);
        pen.font = "700 58px system-ui, sans-serif";
        pen.textAlign = "center";
        pen.textBaseline = "middle";
        pen.fillStyle = "#ffc04d";
        pen.fillText(`+${amount} sats`, 128, 50);
        texture.needsUpdate = true;
      },
    };
  })();
  let struck = -1;
  let nextStrike = 5;
  function strike(now) {
    struck = now;
    nextStrike = now + 8 + random() * 9;
    sats.say([21, 21, 100, 210, 500, 1000, 2100][Math.floor(random() * 7)]);
  }

  // What drifts: fireflies over the grass, and petals off the flower.
  function motes(count, color, size, spread) {
    const positions = new Float32Array(count * 3);
    const seeds = Array.from({ length: count }, () => [random(), random(), random(), random()]);
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    const points = new THREE.Points(geometry, new THREE.PointsMaterial({ color, size, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, fog: false }));
    points.frustumCulled = false;
    scene.add(points);
    return { positions, seeds, attribute: geometry.attributes.position, spread };
  }
  const fireflies = motes(70, new THREE.Color(1.7, 1.5, 0.5), 0.09);
  const petals = motes(14, new THREE.Color(2.2, 0.9, 1.5), 0.11);

  // ── Going up to things ──
  const articles = [...document.querySelectorAll("[data-subject]")].filter((article) => SUBJECTS[article.dataset.subject]);
  const order = articles.map((article) => article.dataset.subject);
  const titles = Object.fromEntries(articles.map((article) => [article.dataset.subject, article.querySelector("h3").textContent]));
  const hits = [];
  const marks = {};
  for (const [id, subject] of Object.entries(SUBJECTS)) {
    const foot = props[subject.props[0]].position;
    subject.foot = new THREE.Vector3(foot.x, 0, foot.z);
    for (const name of subject.props) {
      const box = new THREE.Box3().setFromObject(props[name]).expandByScalar(0.25);
      const proxy = new THREE.Mesh(new THREE.BoxGeometry(...box.getSize(new THREE.Vector3()).toArray()), new THREE.MeshBasicMaterial({ visible: false }));
      box.getCenter(proxy.position);
      proxy.userData.subject = id;
      proxy.updateMatrixWorld();
      hits.push(proxy);
      subject.top ??= new THREE.Vector3(proxy.position.x, box.max.y, proxy.position.z);
    }
    marks[id] = subject.props
      .filter((name) => name !== "cloud")
      .map((name) => {
        const mark = new THREE.Mesh(
          new THREE.RingGeometry(subject.ring - 0.045, subject.ring, 64).rotateX(-Math.PI / 2),
          new THREE.MeshBasicMaterial({ color: PURPLE.clone().multiplyScalar(1.2), transparent: true, opacity: 0.3, depthWrite: false, blending: THREE.AdditiveBlending }),
        );
        mark.position.set(props[name].position.x, 0.03, props[name].position.z);
        scene.add(mark);
        return mark;
      });
  }

  const label = document.createElement("div");
  label.className = "stage-label";
  label.hidden = true;
  const tour = document.getElementById("tour");
  const tourBody = tour.querySelector(".tour-body");
  const tourCount = tour.querySelector(".tour-count");
  const strip = document.getElementById("tour-strip");
  stage.append(canvas, label);

  let selected = null;
  let hovered = null;
  const chips = {};
  for (const id of order) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.textContent = titles[id];
    chip.addEventListener("click", () => select(selected === id ? null : id));
    strip.append(chip);
    chips[id] = chip;
  }
  strip.hidden = false;

  const goal = { from: HOME.from.clone(), look: HOME.look.clone() };
  const eye = { from: HOME.from.clone(), look: HOME.look.clone() };
  let aspect = 1;
  function home() {
    // A narrow window stands further back, to take the same plain in.
    const back = THREE.MathUtils.clamp(1.7 / aspect, 1, 2.4);
    goal.from.set(HOME.from.x, HOME.from.y + (back - 1) * 1.6, HOME.from.z * back);
    goal.look.copy(HOME.look);
  }

  function select(id) {
    selected = id;
    for (const [other, chip] of Object.entries(chips)) chip.setAttribute("aria-pressed", String(other === id));
    tour.hidden = !id;
    stage.classList.toggle("touring", Boolean(id));
    if (!id) {
      home();
      return;
    }
    const subject = SUBJECTS[id];
    const back = THREE.MathUtils.clamp(1.5 / aspect, 1, 1.9);
    goal.from.set(subject.from[0], subject.from[1], subject.from[2] * back).add(subject.foot);
    goal.look.fromArray(subject.look).add(subject.foot);
    tourBody.replaceChildren(...articles.find((article) => article.dataset.subject === id).cloneNode(true).childNodes);
    tourCount.textContent = `${order.indexOf(id) + 1} / ${order.length}`;
    if (id === "zaps") nextStrike = 0;
  }
  function step(by) {
    const at = selected ? order.indexOf(selected) : by > 0 ? -1 : 0;
    select(order[(at + by + order.length) % order.length]);
  }
  tour.querySelector(".tour-close").addEventListener("click", () => select(null));
  for (const button of tour.querySelectorAll("[data-step]")) button.addEventListener("click", () => step(Number(button.dataset.step)));
  addEventListener("keydown", (event) => {
    if (!selected || !running || event.target.closest?.("input, textarea, select")) return;
    if (event.key === "Escape") select(null);
    else if (event.key === "ArrowRight") step(1);
    else if (event.key === "ArrowLeft") step(-1);
  });

  const pointer = new THREE.Vector2(0, 0);
  // Seconds the scene has run for, as of the frame last drawn.
  let now = 0;
  let over = false;
  let moved = 0;
  const ray = new THREE.Raycaster();
  canvas.addEventListener("pointermove", (event) => {
    const box = canvas.getBoundingClientRect();
    pointer.set(((event.clientX - box.left) / box.width) * 2 - 1, -((event.clientY - box.top) / box.height) * 2 + 1);
    over = true;
    moved = now;
  });
  canvas.addEventListener("pointerleave", () => {
    over = false;
    hovered = null;
  });
  function under() {
    ray.setFromCamera(pointer, camera);
    return ray.intersectObjects(hits, false)[0]?.object.userData.subject ?? null;
  }
  canvas.addEventListener("click", (event) => {
    const box = canvas.getBoundingClientRect();
    pointer.set(((event.clientX - box.left) / box.width) * 2 - 1, -((event.clientY - box.top) / box.height) * 2 + 1);
    const id = under();
    select(id === selected ? null : id);
  });

  // ── Drawing it ──
  const target = new THREE.WebGLRenderTarget(1, 1, { type: THREE.HalfFloatType, samples: 4 });
  const composer = new EffectComposer(renderer, target);
  const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.32, 0.6, 1.0);
  composer.addPass(new RenderPass(scene, camera));
  composer.addPass(bloom);
  composer.addPass(new OutputPass());

  function resize() {
    const wide = Math.max(1, stage.clientWidth);
    const tall = Math.max(1, stage.clientHeight);
    const ratio = Math.min(devicePixelRatio, 1.75);
    aspect = wide / tall;
    renderer.setPixelRatio(ratio);
    renderer.setSize(wide, tall, false);
    composer.setPixelRatio(ratio);
    composer.setSize(wide, tall);
    camera.aspect = aspect;
    camera.updateProjectionMatrix();
    night.material.uniforms.scale.value = ratio * Math.max(1, tall / 900);
    if (selected) select(selected);
    else home();
  }
  new ResizeObserver(resize).observe(stage);

  const timer = new THREE.Timer();
  timer.connect(document);
  const facing = new THREE.Matrix4();
  const spot = new THREE.Vector3();
  const matrix = new THREE.Matrix4();
  const projected = new THREE.Vector3();
  // How far what is looked at is put off the middle of the picture, as a
  // share of its width and height: out from under the words about it.
  const aside = new THREE.Vector2();
  const asideGoal = new THREE.Vector2();
  const narrow = matchMedia("(max-width: 56rem)");
  const lookAt = new THREE.Vector3();
  let shown = -1;

  function frame() {
    timer.update();
    const dt = Math.min(timer.getDelta(), 0.1);
    now = timer.getElapsed();

    // The camera eases to where it is going, and leans with the pointer.
    const ease = 1 - Math.exp(-dt * 2.4);
    eye.from.lerp(goal.from, ease);
    eye.look.lerp(goal.look, ease);
    const lean = selected ? 0.25 : 1;
    camera.position.set(eye.from.x + pointer.x * 0.9 * lean + Math.sin(now * 0.11) * 0.25, eye.from.y + pointer.y * 0.3 * lean + Math.sin(now * 0.17) * 0.08, eye.from.z);
    camera.lookAt(lookAt.copy(eye.look));
    // The words about a subject are on the right, or on a narrow screen
    // at the bottom: the subject goes to the middle of what is left.
    aside.lerp(asideGoal.set(selected && !narrow.matches ? 0.15 : 0, selected && narrow.matches ? 0.2 : 0), ease);
    const [wide, tall] = [stage.clientWidth, stage.clientHeight];
    camera.setViewOffset(wide, tall, aside.x * wide, aside.y * tall, wide, tall);

    night.material.uniforms.time.value = now;

    // The bird looks at the pointer, or about itself when there is none.
    const idle = !over || now - moved > 5;
    const yaw = idle ? Math.sin(now * 0.45) * 0.55 + 0.25 : THREE.MathUtils.clamp(-pointer.x * 1.2, -0.9, 1.1) + 0.3;
    const pitch = idle ? Math.sin(now * 0.7) * 0.08 : THREE.MathUtils.clamp(pointer.y * 0.4, -0.3, 0.35);
    head.rotation.y += (yaw - head.rotation.y) * Math.min(1, dt * 4);
    head.rotation.z += (pitch - head.rotation.z) * Math.min(1, dt * 4);
    models.ostrich.position.y = Math.sin(now * 1.3) * 0.012;

    // The relay: its lamp, and its rings going out.
    const blink = Math.max(0, Math.sin(now * 2.2)) ** 6;
    for (const material of beacon) material.emissiveIntensity = 0.4 + blink * 9;
    beaconLight.intensity = blink * 26;
    rings.forEach((ring, i) => {
      const phase = (now * 0.22 + i / rings.length) % 1;
      ring.scale.setScalar(0.4 + phase * 9);
      ring.material.opacity = (1 - phase) ** 1.5 * 0.4;
    });

    // Notes along the lines, each facing whoever looks.
    facing.extractRotation(camera.matrixWorld);
    travelling.forEach((note, i) => {
      if (note.wait > 0 || !note.line) {
        note.wait -= dt;
        if (note.wait <= 0) send(note, i);
        paper.setMatrixAt(i, NOWHERE);
        return;
      }
      note.t += (dt * note.speed * 9) / note.line[note.span].getLength();
      if (note.t >= 1) {
        note.t = 0;
        note.span += note.back ? -1 : 1;
        if (note.span < 0 || note.span >= note.line.length) {
          note.line = null;
          note.wait = 1 + random() * 7;
          paper.setMatrixAt(i, NOWHERE);
          return;
        }
      }
      note.line[note.span].getPointAt(note.back ? 1 - note.t : note.t, spot);
      spot.y += 0.2 + Math.sin(now * 6 + i) * 0.03;
      paper.setMatrixAt(i, matrix.copy(facing).setPosition(spot));
    });
    paper.instanceMatrix.needsUpdate = true;

    // Lightning, now and then, and at once when it is what is looked at.
    if (now > nextStrike) strike(now);
    const since = now - struck;
    const lit = struck >= 0 && (since < 0.09 || (since > 0.16 && since < 0.3));
    const after = struck >= 0 ? Math.max(0, 1 - since / 0.9) : 0;
    bolt.visible = lit;
    flash.intensity = lit ? 260 : after * 30;
    for (const material of stormy) material.emissiveIntensity = lit ? 1.6 : after * 0.5;
    sats.sprite.visible = struck >= 0 && since < 2.2;
    if (sats.sprite.visible) {
      sats.sprite.position.set(PLACES.rod.at[0] + 1.5, 4.2 + since * 0.9, PLACES.rod.at[1]);
      sats.sprite.material.opacity = Math.min(1, (2.2 - since) * 1.5);
    }
    cloud.position.y = 7.3 + Math.sin(now * 0.5) * 0.12;

    // Fireflies wander; petals rise off the flower and are gone.
    fireflies.seeds.forEach(([a, b, c, d], i) => {
      fireflies.positions.set([(a - 0.5) * 30 + Math.sin(now * 0.3 + d * 40) * 1.2, 0.4 + c * 2.4 + Math.sin(now * 0.8 + a * 30) * 0.3, (b - 0.5) * 18 + 1 + Math.cos(now * 0.25 + c * 40) * 1.2], i * 3);
    });
    fireflies.attribute.needsUpdate = true;
    petals.seeds.forEach(([a, b, c, d], i) => {
      const rise = (now * (0.05 + d * 0.05) + a) % 1;
      petals.positions.set([PLACES.blossom.at[0] + (b - 0.5) * 0.5 + Math.sin(now + a * 20) * 0.25 * rise, 1.7 + rise * 3.2, PLACES.blossom.at[1] + 0.3 + (c - 0.5) * 0.5 + Math.cos(now * 0.8 + b * 20) * 0.25 * rise], i * 3);
    });
    petals.attribute.needsUpdate = true;

    // The screen moves on a note at a time.
    const turn = Math.floor(now / 1.7);
    if (turn !== shown) show.draw((shown = turn));

    // What the pointer is on, said over it.
    hovered = over && now - moved < 8 ? under() : null;
    canvas.style.cursor = hovered ? "pointer" : "";
    const named = hovered && hovered !== selected ? hovered : null;
    label.hidden = !named;
    if (named) {
      projected.copy(SUBJECTS[named].top).project(camera);
      label.textContent = titles[named];
      label.style.transform = `translate(-50%, -100%) translate(${((projected.x + 1) / 2) * stage.clientWidth}px, ${((1 - projected.y) / 2) * stage.clientHeight - 12}px)`;
    }
    for (const [id, circles] of Object.entries(marks)) {
      const strength = id === selected ? 0.95 : id === hovered ? 0.8 : 0.22 + 0.1 * Math.sin(now * 1.6 + id.length);
      for (const mark of circles) mark.material.opacity += (strength - mark.material.opacity) * Math.min(1, dt * 6);
    }

    composer.render(dt);
  }

  // Only drawn while it is the scene's page and the scene is in view.
  let wanted = false;
  let visible = true;
  let running = false;
  function update() {
    const run = wanted && visible && !document.hidden;
    if (run === running) return;
    running = run;
    renderer.setAnimationLoop(run ? frame : null);
  }
  new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    update();
  }).observe(stage);
  document.addEventListener("visibilitychange", update);

  resize();
  eye.from.copy(goal.from);
  eye.look.copy(goal.look);
  // `?see=keys` opens on a subject.
  const see = new URLSearchParams(location.search).get("see");
  if (SUBJECTS[see] && order.includes(see)) {
    select(see);
    eye.from.copy(goal.from);
    eye.look.copy(goal.look);
  }
  frame();
  document.documentElement.classList.add("scene-ready");

  return {
    pause() {
      wanted = false;
      update();
    },
    resume() {
      wanted = true;
      update();
    },
  };
}
