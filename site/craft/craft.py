"""The scene's props, from nothing to a file a browser draws.

Each prop is a list of SolidCraft commands (`Design`), run headless to a glTF
file of bare bodies. TextureCraft unwraps it, bakes its mesh maps and lays a
material over each look, and glTF Transform packs the result for the web.
`build.py` drives it; `models.py` holds the props.

Lengths are millimetres with Z up, as SolidCraft has them; the files written
are metres with Y up, as glTF has them.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

Point = tuple[float, float, float]


@dataclass(frozen=True)
class Look:
    """What a body is made of: a colour for SolidCraft's own pictures, and
    what TextureCraft lays over it."""

    color: str
    # A TextureCraft smart material, by id (`texturecraft-cli run … material.list`).
    material: str | None = None
    # Or one plain fill, for a colour no smart material has.
    roughness: float = 0.6
    metallic: float = 0.0
    # Dark in the creases, as things left outdoors are: 0 for none.
    dirt: float = 0.0
    # The side of its texture, in pixels.
    size: int = 512


@dataclass
class Design:
    """A SolidCraft script in the making."""

    commands: list[dict] = field(default_factory=list)
    sketches: int = 0
    looks: dict[str, list[str]] = field(default_factory=dict)

    def run(self, command: str, **params) -> None:
        self.commands.append({"command": command, "params": {k: v for k, v in params.items() if v is not None}})

    # ── Primitives ──────────────────────────────────────────────────

    def box(self, name: str, size: Point, center: Point, look: str | None = None, **more) -> str:
        self.run("solid.box", length=size[0], width=size[1], height=size[2], center=list(center), body_name=name, **more)
        return self._made(name, look, more)

    def cylinder(self, name: str, radius: float, height: float, base: Point = (0, 0, 0), axis="Z", look=None, **more) -> str:
        self.run("solid.cylinder", radius=radius, height=height, base=list(base), axis=axis, body_name=name, **more)
        return self._made(name, look, more)

    def cone(self, name: str, radius: float, height: float, base: Point = (0, 0, 0), axis="Z", top: float = 0, look=None, **more) -> str:
        self.run("solid.cone", radius=radius, height=height, top_radius=top or None, base=list(base), axis=axis, body_name=name, **more)
        return self._made(name, look, more)

    def sphere(self, name: str, radius: float, center: Point, look=None, **more) -> str:
        self.run("solid.sphere", radius=radius, center=list(center), body_name=name, **more)
        return self._made(name, look, more)

    def ellipsoid(self, name: str, radii: Point, center: Point, look=None) -> str:
        """A sphere pulled to three radii, along the axes."""
        self.sphere(name, radii[0], center, look)
        self.run("solid.scale", bodies=[name], factors=[1, radii[1] / radii[0], radii[2] / radii[0]], origin=list(center))
        return name

    def torus(self, name: str, major: float, minor: float, center: Point, look=None, **more) -> str:
        self.run("solid.torus", major=major, minor=minor, center=list(center), body_name=name, **more)
        return self._made(name, look, more)

    # ── From sketches ───────────────────────────────────────────────

    def sketch(self, plane: str, offset: float = 0) -> str:
        """Start a sketch and return its name. Points given to the sketch
        helpers below are (x, y) on XY, (y, z) on YZ and (x, z) on XZ."""
        self.run("sketch.create", plane=plane, offset=offset or None)
        self.sketches += 1
        self.plane = plane
        return f"Sketch{self.sketches}"

    def _on_plane(self, points):
        # SolidCraft's XZ sketch has its y along world -Z.
        return [[a, -b] if self.plane == "XZ" else [a, b] for a, b in points]

    def revolve(self, name: str, profile: list[tuple[float, float]], look=None, **more) -> str:
        """Turn a closed outline of (radius, z) points about Z."""
        self.sketch("YZ")
        self.run("sketch.line", points=self._on_plane(profile), closed=True)
        self.run("sketch.finish")
        self.run("solid.revolve", axis="Z", body_name=name, **more)
        return self._made(name, look, more)

    def extrude(self, name: str, plane: str, outline: list[tuple[float, float]], distance: float, offset: float = 0, look=None, **more) -> str:
        """Push a closed outline out of its plane, from `offset` along the
        plane's normal: +Z for XY, +X for YZ, +Y for XZ."""
        self.sketch(plane, offset)
        self.run("sketch.line", points=self._on_plane(outline), closed=True)
        self.run("sketch.finish")
        self.run("solid.extrude", distance=distance, body_name=name, **more)
        return self._made(name, look, more)

    def pipe(self, name: str, plane: str, path: list[tuple[float, float]], diameter: float, offset: float = 0, smooth=False, look=None, **more) -> str:
        """A round bar along a path in a plane: through its points with
        corners, or smoothly."""
        sketch = self.sketch(plane, offset)
        points = self._on_plane(path)
        if smooth:
            self.run("sketch.spline.fit_point", points=points)
            curves = ["s1"]
        else:
            self.run("sketch.line", points=points)
            curves = [f"l{i + 1}" for i in range(len(points) - 1)]
        self.run("sketch.finish")
        self.run("solid.pipe", path_sketch=sketch, path=curves, diameter=diameter, body_name=name, **more)
        return self._made(name, look, more)

    # ── Changing what is there ──────────────────────────────────────

    def move(self, bodies, translate: Point | None = None, axis: Point | None = None, angle: float | None = None, origin: Point | None = None, copy=False, names=None):
        self.run(
            "solid.move",
            bodies=[bodies] if isinstance(bodies, str) else list(bodies),
            translate=list(translate) if translate else None,
            axis=list(axis) if axis else None,
            angle=angle,
            origin=list(origin) if origin else None,
            copy=copy or None,
            body_names=names,
        )

    def fillet(self, body: str, edges: list[Point], radius: float) -> None:
        self.run("solid.fillet", edges=[list(e) for e in edges], radius=radius, body=body)

    def join(self, target: str, *tools: str) -> None:
        self.run("solid.combine", target=target, tools=list(tools), operation="join", touching="one_body")
        for members in self.looks.values():
            for tool in tools:
                if tool in members:
                    members.remove(tool)

    def cut(self, target: str, *tools: str, keep=False) -> None:
        self.run("solid.combine", target=target, tools=list(tools), operation="cut", keep_tools=keep or None)
        if not keep:
            for members in self.looks.values():
                for tool in tools:
                    if tool in members:
                        members.remove(tool)

    def look(self, look: str, *bodies: str) -> None:
        for members in self.looks.values():
            for body in bodies:
                if body in members:
                    members.remove(body)
        self.looks.setdefault(look, []).extend(bodies)

    def _made(self, name: str, look: str | None, more: dict) -> str:
        # A body cut out of another, or joined to it, is no body of its own.
        if look and more.get("operation") in (None, "new"):
            self.look(look, name)
        return name

    def script(self, looks: dict[str, Look]) -> list[dict]:
        """The commands, with each look's colour given to its bodies under
        the look's name: that name is the glTF material's, which is how
        TextureCraft knows what to lay over it."""
        commands = list(self.commands)
        for look, bodies in self.looks.items():
            if bodies:
                commands.append({"command": "appearance.assign", "params": {"bodies": bodies, "color": looks[look].color, "name": look}})
        return commands


def tool(name: str) -> str:
    path = shutil.which(name)
    if not path:
        sys.exit(f"{name} is not installed: the scene's props are made with the ArtCraft suite (see site/craft/README.md)")
    return path


def call(*command: str, quiet=True) -> str:
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"{' '.join(command[:3])} … failed:\n{done.stdout[-2000:]}\n{done.stderr[-2000:]}")
    return done.stdout


def model(design: Design, looks: dict[str, Look], bare: Path, picture: Path | None = None) -> None:
    """SolidCraft: the script to a glTF file of bare bodies, and a picture
    of it to check by eye."""
    script = bare.with_suffix(".json")
    script.write_text(json.dumps(design.script(looks), indent=1))
    call(tool("solidcraft-cli"), "run", str(script), "--out", str(bare), "--quiet")
    if picture:
        call(tool("solidcraft-cli"), "snapshot", str(script), "--out", str(picture), "--width", "480", "--height", "480", "--no-grid", "--no-axes")


def texture(looks: dict[str, Look], bare: Path, textured: Path) -> None:
    """TextureCraft: unwrap, bake the mesh maps, lay each look's material
    over its texture set, and write the mesh with its textures."""
    cli = tool("texturecraft-cli")
    sets = json.loads(call(cli, "info", str(bare)))["textureSets"]
    steps: list[dict] = [{"id": "textureSet.resolution", "params": {"ts": i, "resolution": looks[name].size}} for i, name in enumerate(sets)]
    steps.append({"id": "bake.run", "params": {"ts": "all"}})
    for i, name in enumerate(sets):
        look = looks[name]
        steps.append({"id": "textureSet.select", "params": {"ts": i}})
        if look.material:
            steps.append({"id": "material.apply", "params": {"material": look.material}})
        else:
            steps.append({"id": "layer.add.fill", "params": {"name": name, "color": look.color, "roughness": look.roughness, "metallic": look.metallic}})
        if look.dirt:
            steps.append({"id": "layer.add.fill", "params": {"name": "dirt", "color": "#1a1410", "roughness": 0.9, "metallic": 0.0}})
            steps.append({"id": "effect.add.generator", "params": {"generator": "dirt", "level": look.dirt}})
    steps.append({"id": "file.exportMesh", "params": {"path": str(textured.resolve())}})
    batch = textured.with_suffix(".batch.json")
    batch.write_text(json.dumps({"open": str(bare.resolve()), "steps": steps}, indent=1))
    report = json.loads(call(cli, "batch", str(batch)))
    if report.get("failed") is not None:
        sys.exit(f"TextureCraft stopped at step {report['failed']} of {batch}")


def pack(transform: str, textured: Path, packed: Path, largest: int) -> None:
    """glTF Transform: fewer triangles where nothing is lost for it,
    meshopt-compressed geometry and WebP textures."""
    call(
        transform, "optimize", str(textured), str(packed),
        "--compress", "meshopt", "--texture-compress", "webp", "--texture-size", str(largest),
        "--simplify", "true", "--simplify-error", "0.003",
        "--join", "false", "--flatten", "false", "--palette", "false", "--instance", "false",
    )


def tile(material: str, size: int, work: Path, out: Path, name: str) -> None:
    """TextureCraft: one of its materials on a flat square, which repeats
    edge to edge, as the colour and the normal map of the ground. PhotoCraft
    writes them out for the web."""
    cli = tool("texturecraft-cli")
    project, maps = work / f"{name}.tcraft", work / f"{name}-maps"
    call(cli, "new", "builtin:plane", str(project), "--resolution", str(size))
    call(cli, "run", str(project), "material.apply", json.dumps({"material": material}), "-o", str(project))
    call(cli, "export", str(project), str(maps), "--preset", "glTF PBR Metal Roughness", "--format", "webp", "--size", str(size))
    out.mkdir(parents=True, exist_ok=True)
    # PhotoCraft: as small as they go without it showing.
    for kind, quality in (("BaseColor", "80"), ("Normal", "88")):
        target = out / f"{name}_{'color' if kind == 'BaseColor' else 'normal'}.webp"
        call(tool("photocraft-cli"), "convert", str(maps / f"plane_Plane_{kind}.webp"), str(target), "--quality", quality, "--metadata", "none")
