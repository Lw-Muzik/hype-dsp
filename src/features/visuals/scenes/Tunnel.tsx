import { useRef } from "react";
import { GOLD, GREEN, THREE, useThreeScene } from "./threeKit";

const RINGS = 60;
const DEPTH = 60; // how far back the tunnel extends
const NEAR = 4; // recycle once a ring passes this z

/** Tunnel (3D) — fly through a wormhole of rings that pulse and shift hue with
 *  the beat; travel speed scales with overall energy. */
export function Tunnel() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useThreeScene(canvasRef, ({ scene, camera }) => {
    camera.position.set(0, 0, NEAR);
    camera.lookAt(0, 0, -10);

    const geo = new THREE.TorusGeometry(1.6, 0.05, 8, 40);
    const mat = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.9 });
    const mesh = new THREE.InstancedMesh(geo, mat, RINGS);

    const z = new Float32Array(RINGS);
    const m = new THREE.Matrix4();
    const col = new THREE.Color();
    for (let i = 0; i < RINGS; i++) {
      z[i] = NEAR - (i / RINGS) * DEPTH;
    }

    const place = (i: number, time: number, beat: number) => {
      const zi = z[i] ?? 0;
      // Snake the tunnel a little so it reads as a wormhole.
      const x = Math.sin(zi * 0.12 + time * 0.4) * 0.8;
      const y = Math.cos(zi * 0.1 + time * 0.3) * 0.6;
      const s = 1 + beat * 0.25;
      m.makeScale(s, s, 1);
      m.setPosition(x, y, zi);
      mesh.setMatrixAt(i, m);
      const depth = (NEAR - zi) / DEPTH; // 0 near → 1 far
      col.copy(GOLD).lerp(GREEN, depth);
      mesh.setColorAt(i, col);
    };

    let time = 0;
    for (let i = 0; i < RINGS; i++) place(i, 0, 0);
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    scene.add(mesh);

    return {
      frame: (a, dt) => {
        time += dt;
        const speed = (6 + a.level * 22 + a.beat * 14) * dt;
        for (let i = 0; i < RINGS; i++) {
          z[i] = (z[i] ?? 0) + speed;
          if ((z[i] ?? 0) > NEAR) z[i] = (z[i] ?? 0) - DEPTH; // recycle to the back
          place(i, time, a.beat);
        }
        mesh.instanceMatrix.needsUpdate = true;
        if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
      },
      dispose: () => {
        geo.dispose();
        mat.dispose();
        mesh.dispose();
      },
    };
  });

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
