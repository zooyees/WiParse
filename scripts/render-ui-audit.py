"""Render egui audit meshes/font atlases without a window or GPU (visual QA only)."""
import json, sys
from pathlib import Path
import numpy as np
from PIL import Image

root = Path(sys.argv[1])
sources = [root / (name + '.json') for name in sys.argv[2:]] if len(sys.argv) > 2 else root.glob('*.json')
for source in sources:
    data = json.loads(source.read_text())
    width, height = map(int, data['size'])
    canvas = np.zeros((height, width, 3), np.float32)
    textures = {t['id']: np.asarray(Image.open(root / t['file']).convert('RGBA'), dtype=np.float32) / 255
                for t in data['textures']}
    for mesh in data['meshes']:
        texture = textures.get(mesh['texture'])
        if texture is None:
            raise RuntimeError('Missing texture ' + mesh['texture'])
        vertices = np.asarray(mesh['vertices'], dtype=np.float32)
        clip = mesh['clip']
        for indices in np.asarray(mesh['indices']).reshape(-1, 3):
            tri = vertices[indices]
            x0 = max(0, int(np.floor(tri[:, 0].min())), int(np.ceil(clip[0])))
            y0 = max(0, int(np.floor(tri[:, 1].min())), int(np.ceil(clip[1])))
            x1 = min(width, int(np.ceil(tri[:, 0].max())), int(np.floor(clip[2])))
            y1 = min(height, int(np.ceil(tri[:, 1].max())), int(np.floor(clip[3])))
            if x1 <= x0 or y1 <= y0:
                continue
            a, b, c = tri[:, :2]
            denominator = (b[1]-c[1])*(a[0]-c[0]) + (c[0]-b[0])*(a[1]-c[1])
            if abs(denominator) < 1e-7:
                continue
            yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
            xx += 0.5
            yy += 0.5
            wa = ((b[1]-c[1])*(xx-c[0]) + (c[0]-b[0])*(yy-c[1]))/denominator
            wb = ((c[1]-a[1])*(xx-c[0]) + (a[0]-c[0])*(yy-c[1]))/denominator
            wc = 1-wa-wb
            mask = (wa >= 0) & (wb >= 0) & (wc >= 0)
            if not mask.any():
                continue
            weights = np.stack([wa[mask], wb[mask], wc[mask]], axis=1)
            uv = weights @ tri[:, 2:4]
            color = (weights @ tri[:, 4:8])/255
            tx = np.clip(uv[:, 0]*texture.shape[1]-0.5, 0, texture.shape[1]-1)
            ty = np.clip(uv[:, 1]*texture.shape[0]-0.5, 0, texture.shape[0]-1)
            ix, iy = tx.astype(int), ty.astype(int)
            jx, jy = np.minimum(ix+1, texture.shape[1]-1), np.minimum(iy+1, texture.shape[0]-1)
            fx, fy = (tx-ix)[:, None], (ty-iy)[:, None]
            sample = (texture[iy, ix]*(1-fx)+texture[iy, jx]*fx)*(1-fy) + (texture[jy, ix]*(1-fx)+texture[jy, jx]*fx)*fy
            # Both egui colors and font atlas use premultiplied alpha.
            src = sample * color
            target = canvas[y0:y1, x0:x1]
            target[mask] = src[:, :3] + target[mask]*(1-src[:, 3:4])
    Image.fromarray((np.clip(canvas, 0, 1)*255).astype(np.uint8)).save(root / (source.stem+'.png'))
    print(source.stem)
