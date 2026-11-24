"""The props of the scene: what stands on the plain.

Each is drawn the size it would be, in millimetres, standing on Z = 0 about
its own origin and looking along +X, so that seen from -Y it is in profile.
`scene/world.js` says where each one stands.
"""

from __future__ import annotations

import math

from craft import Design, Look, Point

LOOKS: dict[str, Look] = {
    # The bird.
    "feather": Look("#2c2d44", roughness=0.92, size=512),
    "plume": Look("#ece6da", roughness=0.95, size=256),
    "skin": Look("#d9b3a4", material="skin", size=256),
    "beak": Look("#d99a45", roughness=0.45, size=128),
    "eye": Look("#0b0b0f", roughness=0.08, size=128),
    # What things are built of.
    "trunk": Look("#5a4634", material="bark", size=512),
    "plank": Look("#a89a86", material="wood_weathered", size=512),
    "pine": Look("#d8b074", material="wood_pine", size=256),
    "zinc": Look("#aeb4b8", material="galvanized", size=1024),
    "iron": Look("#3a3d42", material="cast_iron", size=256),
    "darksteel": Look("#2b2e33", material="steel_dark", size=512),
    "rusty": Look("#7a4a36", material="iron_rusted", size=256),
    "redpaint": Look("#c5362b", material="steel_painted_worn", size=512),
    "brass": Look("#b79b4a", material="gold_dirty", size=512),
    "copper": Look("#b77a5f", material="copper_patina", size=256),
    "concrete": Look("#9a968e", material="concrete_dirty", size=1024),
    "stone": Look("#8d8a80", material="stone_mossy", size=512),
    "sandstone": Look("#d2b684", material="sandstone", size=512),
    "clay": Look("#b96a43", material="clay", size=256),
    "soil": Look("#4a3524", material="mud", size=128),
    "leather": Look("#6a3a22", material="leather_worn", size=256),
    "paper": Look("#efe9dc", material="paper", size=128),
    "porcelain": Look("#f4f2ec", roughness=0.15, size=128),
    "dish": Look("#e4e2dc", roughness=0.5, dirt=0.5, size=256),
    # What grows.
    "leaves": Look("#56702e", material="moss", size=512),
    "stem": Look("#4c7a34", roughness=0.7, size=128),
    "petal": Look("#f08fb8", roughness=0.55, size=256),
    "pollen": Look("#f2c230", roughness=0.8, size=128),
    "ribbon": Look("#c5362b", roughness=0.6, size=128),
    # What shines: the scene lights these itself, by their names.
    "lamp": Look("#ff5b3a", roughness=0.3, size=128),
    "bolt": Look("#ffc247", roughness=0.4, size=128),
    "cloud": Look("#7c7f8c", roughness=1.0, size=256),
}


# Where the bird's neck ends, and its head sits.
NECK_END: Point = (690, 0, 2620)


def bar(design: Design, name: str, start: Point, end: Point, radius: float, look: str) -> str:
    """A round bar from one point to another."""
    along = tuple(b - a for a, b in zip(start, end))
    return design.cylinder(name, radius, math.dist(start, end), base=start, axis=list(along), look=look)


def ostrich() -> Design:
    """The bird itself, less its head: that turns, so it is a prop of its
    own (`ostrich_head`), set on the neck's end at `NECK_END`."""
    d = Design()
    d.ellipsoid("body", (560, 300, 370), (0, 0, 1480), "feather")
    d.move("body", axis=(0, 1, 0), angle=-10, origin=(0, 0, 1480))
    d.ellipsoid("chest", (260, 230, 280), (380, 0, 1520), "feather")
    d.ellipsoid("rump", (300, 240, 260), (-440, 0, 1500), "feather")
    d.ellipsoid("tail", (270, 170, 210), (-720, 0, 1540), "plume")
    d.move("tail", axis=(0, 1, 0), angle=25, origin=(-720, 0, 1540))
    for side, y in (("left", 1), ("right", -1)):
        d.ellipsoid(f"wing_{side}", (400, 60, 230), (-80, 285 * y, 1520), "feather")
        d.ellipsoid(f"wingtip_{side}", (230, 55, 130), (-430, 270 * y, 1400), "plume")
        d.ellipsoid(f"thigh_{side}", (150, 105, 300), (10 + 50 * y, 140 * y, 1180), "feather")
    d.pipe("neck", "XZ", [(470, 1620), (640, 1900), (600, 2250), (690, NECK_END[2])], 105, smooth=True, look="skin")
    d.sphere("neck_end", 56, NECK_END, "skin")
    # One leg forward, one back: it is walking. The joint halfway down an
    # ostrich's leg bends backwards: it is its ankle.
    d.pipe("shank_left", "XZ", [(90, 1010), (40, 560), (230, 60)], 58, offset=140, look="skin")
    d.pipe("shank_right", "XZ", [(-20, 1010), (-200, 600), (-150, 60)], 58, offset=-140, look="skin")
    for side, ankle, toe, y in (("left", 40, 230, 140), ("right", -200, -150, -140)):
        d.sphere(f"ankle_{side}", 46, (ankle, y, 560 if side == "left" else 600), "skin")
        d.ellipsoid(f"foot_{side}", (190, 62, 38), (toe + 120, y, 36), "skin")
    return d


def ostrich_head() -> Design:
    """The bird's head, about the point it turns on."""
    d = Design()
    d.ellipsoid("head", (140, 100, 105), (50, 0, 50), "skin")
    d.cone("beak", 62, 210, base=(150, 0, 35), axis="X", look="beak")
    d.run("solid.scale", bodies=["beak"], factors=[1, 1, 0.55], origin=[150, 0, 35])
    for side, y in (("left", 1), ("right", -1)):
        d.sphere(f"eye_{side}", 34, (85, 78 * y, 80), "eye")
    d.ellipsoid("crest", (70, 42, 30), (0, 0, 135), "plume")
    return d


def tower() -> Design:
    """A relay: a lattice mast with dishes, and a lamp on top."""
    d = Design()
    d.box("footing", (1500, 1500, 260), (0, 0, 130), "concrete")
    foot, top, low, high = 600.0, 170.0, 260.0, 8200.0

    def corner(z: float, sx: int, sy: int) -> Point:
        reach = foot + (top - foot) * (z - low) / (high - low)
        return (sx * reach, sy * reach, z)

    corners = [(1, 1), (-1, 1), (-1, -1), (1, -1)]
    for i, (sx, sy) in enumerate(corners):
        bar(d, f"leg_{i}", corner(low, sx, sy), corner(high, sx, sy), 46, "zinc")
    levels = [low + (high - low) * step / 4 for step in range(5)]
    for level, z in enumerate(levels[1:], start=1):
        for i, (here, there) in enumerate(zip(corners, corners[1:] + corners[:1])):
            bar(d, f"ring_{level}_{i}", corner(z, *here), corner(z, *there), 28, "zinc")
    for level, (below, above) in enumerate(zip(levels, levels[1:])):
        for i, (here, there) in enumerate(zip(corners, corners[1:] + corners[:1])):
            # Each face braced one way, the next the other, section by section.
            a, b = (here, there) if (level + i) % 2 else (there, here)
            bar(d, f"brace_{level}_{i}", corner(below, *a), corner(above, *b), 20, "zinc")
    d.cylinder("deck", 330, 60, base=(0, 0, high), look="zinc")
    d.cylinder("mast", 36, 1500, base=(0, 0, high + 60), look="zinc")
    d.sphere("beacon", 95, (0, 0, high + 1640), "lamp")
    for i, (z, turn) in enumerate(((6400, 20), (7300, 200))):
        name = f"dish_{i}"
        d.revolve(name, [(0, 0), (430, 150), (430, 175), (0, 28)], "dish")
        d.cylinder(f"feed_{i}", 14, 260, base=(0, 0, 20), look="iron")
        d.sphere(f"horn_{i}", 34, (0, 0, 290), "iron")
        group = [name, f"feed_{i}", f"horn_{i}"]
        # Laid on its side to face +X, hung on the mast's side, turned about it.
        d.move(group, axis=(0, 1, 0), angle=90, origin=(0, 0, 0))
        d.move(group, translate=(260, 0, z))
        d.move(group, axis=(0, 0, 1), angle=turn, origin=(0, 0, 0))
    return d


def pole() -> Design:
    """A pole that carries the lines notes travel on. The lines hang from
    the tops of its four insulators: (0, ±300 and ±700, 5235)."""
    d = Design()
    d.cone("post", 105, 5600, top=75, look="trunk")
    d.box("arm", (110, 1700, 120), (0, 0, 5050), "plank")
    bar(d, "strut_a", (0, 0, 4500), (0, 560, 5000), 22, "iron")
    bar(d, "strut_b", (0, 0, 4500), (0, -560, 5000), 22, "iron")
    for i, y in enumerate((-700, -300, 300, 700)):
        d.cylinder(f"pin_{i}", 16, 60, base=(0, y, 5110), look="iron")
        d.revolve(f"cup_{i}", [(0, 0), (48, 0), (56, 30), (40, 50), (52, 75), (30, 110), (0, 110)], "porcelain")
        d.move(f"cup_{i}", translate=(0, y, 5125))
    return d


def bunker() -> Design:
    """Where a key is kept: a pillbox with a hatch, and a key as tall as
    its wall leant on it."""
    d = Design()
    d.revolve("shell", [(0, 0), (1350, 0), (1350, 180), (1180, 820), (940, 1000), (0, 1000)], "concrete")
    # The slit it looks out of, towards -Y.
    d.box("slit", (1100, 700, 130), (0, -1000, 640), operation="cut", targets=["shell"])
    d.cylinder("collar", 470, 70, base=(0, 0, 1000), look="concrete")
    d.cylinder("hatch", 400, 60, base=(0, 0, 1070), look="iron")
    d.cylinder("spindle", 28, 150, base=(0, 0, 1130), look="iron")
    d.torus("wheel", 190, 20, (0, 0, 1260), "iron")
    for i, turn in enumerate((0, 60, 120)):
        d.box(f"spoke_{i}", (380, 26, 18), (0, 0, 1260), "iron")
        d.move(f"spoke_{i}", axis=(0, 0, 1), angle=turn, origin=(0, 0, 0))
    # The key, drawn lying along X from its bow, half as large again,
    # then stood on its bit and leant on the wall, to the left of the slit.
    key = ["bow", "shank", "bit_a", "bit_b", "ring_a", "ring_b"]
    d.torus("bow", 200, 52, (0, 0, 0), "brass")
    d.cylinder("shank", 44, 1050, base=(190, 0, 0), axis="X", look="brass")
    d.torus("ring_a", 58, 16, (0, 0, 0), "brass")
    d.torus("ring_b", 58, 16, (0, 0, 0), "brass")
    d.move(["ring_a", "ring_b"], axis=(0, 1, 0), angle=90, origin=(0, 0, 0))
    d.move("ring_a", translate=(330, 0, 0))
    d.move("ring_b", translate=(410, 0, 0))
    d.box("bit_a", (150, 230, 46), (960, -150, 0), "brass")
    d.box("bit_b", (110, 150, 46), (1150, -110, 0), "brass")
    d.run("solid.scale", bodies=key, factor=1.5, origin=[0, 0, 0])
    d.move(key, axis=(0, 1, 0), angle=62, origin=(0, 0, 0))
    d.move(key, axis=(0, 0, 1), angle=180, origin=(0, 0, 0))
    d.move(key, translate=(-1180, 0, 1700))
    d.move(key, axis=(0, 0, 1), angle=58, origin=(0, 0, 0))
    return d


def rod() -> Design:
    """Where lightning comes down: a copper rod on a heap of rocks."""
    d = Design()
    d.ellipsoid("rock_a", (900, 700, 520), (0, 0, 180), "stone")
    d.ellipsoid("rock_b", (560, 480, 380), (-700, 300, 120), "stone")
    d.ellipsoid("rock_c", (420, 520, 300), (620, -420, 90), "stone")
    d.cylinder("staff", 26, 2900, base=(0, 0, 600), look="copper")
    d.sphere("ball", 85, (0, 0, 3000), "copper")
    d.cone("tip", 26, 420, base=(0, 0, 3500), look="copper")
    d.torus("band", 60, 14, (0, 0, 900), "copper")
    return d


def bolt() -> Design:
    """Lightning, as a zap is drawn: its foot at the origin."""
    d = Design()
    outline = [(150, 1900), (-330, 780), (-40, 780), (-200, 0), (380, 1130), (60, 1130), (290, 1900)]
    d.extrude("bolt", "XZ", outline, 90, offset=-45, look="bolt")
    return d


def cloud() -> Design:
    """The cloud lightning comes out of, about its own middle."""
    d = Design()
    puffs = [((900, 620, 480), (0, 0, 0)), ((640, 520, 420), (-820, 80, -60)), ((700, 540, 440), (800, -60, -40)),
             ((520, 460, 400), (-300, 60, 330)), ((560, 480, 420), (380, -40, 300)), ((420, 380, 300), (-1350, 0, -140)),
             ((440, 400, 300), (1380, 20, -130))]
    for i, (radii, center) in enumerate(puffs):
        d.ellipsoid(f"puff_{i}", radii, center, "cloud")
    return d


def blossom() -> Design:
    """A flower in a pot: pictures and videos are kept on Blossom servers."""
    d = Design()
    d.revolve("pot", [(0, 0), (210, 0), (300, 420), (330, 420), (330, 500), (270, 500), (190, 40), (0, 40)], "clay")
    d.cylinder("earth", 268, 30, base=(0, 0, 430), look="soil")
    d.pipe("stalk", "XZ", [(0, 440), (70, 800), (-30, 1200), (20, 1620)], 46, smooth=True, look="stem")
    for i, (z, turn) in enumerate(((760, 30), (1050, 200))):
        d.ellipsoid(f"leaf_{i}", (230, 95, 22), (250, 0, z), "stem")
        d.move(f"leaf_{i}", axis=(0, 1, 0), angle=-28, origin=(40, 0, z))
        d.move(f"leaf_{i}", axis=(0, 0, 1), angle=turn, origin=(20, 0, 0))
    head: Point = (20, 0, 1660)
    d.ellipsoid("heart", (130, 130, 70), head, "pollen")
    petals = []
    for i in range(8):
        name = f"petal_{i}"
        d.ellipsoid(name, (190, 100, 26), (head[0] + 230, 0, head[2]), "petal")
        # Each lifted a little at its tip, then set round the heart.
        d.move(name, axis=(0, 1, 0), angle=-22, origin=(head[0] + 60, 0, head[2]))
        d.move(name, axis=(0, 0, 1), angle=45 * i, origin=head)
        petals.append(name)
    # The whole flower leans towards whoever looks, from -Y.
    d.move(["heart", *petals], axis=(1, 0, 0), angle=38, origin=head)
    return d


def mailbox() -> Design:
    """What is written waits here to be sent: a mailbox with its flag up."""
    d = Design()
    d.box("post", (110, 110, 1150), (0, 0, 575), "trunk")
    d.box("shelf", (560, 260, 40), (0, 0, 1170), "plank")
    d.box("box", (520, 240, 150), (0, 0, 1265), "redpaint")
    d.cylinder("vault", 120, 520, base=(-260, 0, 1340), axis="X", look="redpaint")
    d.join("box", "vault")
    d.cylinder("door", 124, 16, base=(260, 0, 1340), axis="X", look="redpaint")
    d.sphere("knob", 20, (284, 0, 1400), "brass")
    d.box("staff", (18, 14, 260), (120, -128, 1420), "iron")
    d.box("flag", (150, 12, 100), (190, -130, 1500), "redpaint")
    # A note half posted, in the slot on top.
    d.box("note", (190, 6, 150), (-60, 0, 1500), "paper")
    return d


def billboard() -> Design:
    """A screen on two posts, with lamps over it. What it shows is laid
    over its face by the scene: 3000 by 1800, centred on (0, -75, 2700)."""
    d = Design()
    for i, x in enumerate((-1250, 1250)):
        d.cone(f"post_{i}", 95, 3700, base=(x, 60, 0), top=80, look="trunk")
    d.box("frame", (3240, 110, 2040), (0, 0, 2700), "darksteel")
    d.box("face", (3000, 40, 1800), (0, -55, 2700), operation="cut", targets=["frame"])
    d.box("walk", (3240, 420, 40), (0, -180, 1640), "plank")
    for i, x in enumerate((-1000, 0, 1000)):
        d.pipe(f"neck_{i}", "YZ", [(0, 3720), (0, 3900), (-420, 3980)], 28, offset=x, look="iron")
        d.cone(f"shade_{i}", 95, 130, base=(x, -470, 3930), axis=[0, 0.35, 1], top=30, look="iron")
        d.sphere(f"bulb_{i}", 42, (x, -470, 3930), "lamp")
    return d


def telescope() -> Design:
    """What one looks for things with: a brass telescope on three legs."""
    d = Design()
    apex: Point = (0, 0, 1350)
    for i in range(3):
        angle = math.radians(90 + 120 * i)
        bar(d, f"leg_{i}", (560 * math.cos(angle), 560 * math.sin(angle), 0), apex, 28, "pine")
    d.sphere("head", 70, apex, "brass")
    along = (math.cos(math.radians(32)), 0.0, math.sin(math.radians(32)))

    def at(distance: float) -> Point:
        return tuple(a + distance * b for a, b in zip(apex, along))

    lift: Point = (0, 0, 105)
    raised = lambda p: tuple(a + b for a, b in zip(p, lift))
    d.cylinder("tube", 72, 900, base=raised(at(-380)), axis=list(along), look="brass")
    d.cylinder("draw", 54, 300, base=raised(at(-640)), axis=list(along), look="brass")
    d.cylinder("eyepiece", 34, 120, base=raised(at(-740)), axis=list(along), look="iron")
    d.cylinder("hood", 88, 150, base=raised(at(480)), axis=list(along), look="brass")
    d.cylinder("post", 24, 110, base=apex, look="brass")
    return d


def book() -> Design:
    """What one keeps: a book with a ribbon in it, on a rock."""
    d = Design()
    d.ellipsoid("rock", (760, 620, 330), (0, 0, 130), "sandstone")
    z = 465
    d.box("back", (560, 760, 26), (0, 0, z), "leather")
    d.box("pages", (520, 720, 120), (-10, 0, z + 73), "paper")
    d.box("front", (560, 760, 26), (0, 0, z + 146), "leather")
    d.box("spine", (30, 760, 172), (-282, 0, z + 73), "leather")
    d.box("mark", (46, 300, 8), (60, -470, z + 100), "ribbon")
    d.box("mark_end", (46, 8, 150), (60, -618, z + 28), "ribbon")
    book = ["back", "pages", "front", "spine", "mark", "mark_end"]
    d.move(book, axis=(0, 0, 1), angle=22, origin=(0, 0, 0))
    return d


def acacia() -> Design:
    """A flat-topped tree, as the plain has them."""
    d = Design()
    d.pipe("bole", "XZ", [(0, 0), (120, 1300), (-80, 2500), (60, 3400)], 420, smooth=True, look="trunk")
    d.pipe("limb_a", "XZ", [(-40, 2300), (-900, 3300), (-1900, 3900)], 220, smooth=True, look="trunk")
    d.pipe("limb_b", "XZ", [(20, 2600), (900, 3400), (2000, 3950)], 240, smooth=True, look="trunk")
    d.pipe("limb_c", "YZ", [(0, 2700), (700, 3400), (1500, 3900)], 200, smooth=True, look="trunk")
    crowns = [((2300, 1900, 420), (0, 200, 4200)), ((1700, 1400, 360), (-1700, -200, 4150)),
              ((1800, 1500, 380), (1800, 0, 4250)), ((1500, 1300, 340), (200, 1500, 4180)),
              ((1300, 1100, 300), (300, -1200, 4380))]
    for i, (radii, center) in enumerate(crowns):
        d.ellipsoid(f"crown_{i}", radii, center, "leaves")
    return d


def rock() -> Design:
    """A rock, to be strewn about at any size."""
    d = Design()
    d.ellipsoid("rock", (520, 400, 300), (0, 0, 120), "stone")
    d.ellipsoid("lump", (300, 330, 240), (330, 120, 90), "stone")
    d.join("rock", "lump")
    return d


MODELS = {
    "ostrich": ostrich, "ostrich_head": ostrich_head, "tower": tower, "pole": pole, "bunker": bunker,
    "rod": rod, "bolt": bolt, "cloud": cloud, "blossom": blossom, "mailbox": mailbox,
    "billboard": billboard, "telescope": telescope, "book": book, "acacia": acacia, "rock": rock,
}
