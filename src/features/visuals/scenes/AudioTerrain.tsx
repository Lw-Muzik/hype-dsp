import { useRef } from "react";
import { THREE, useThreeScene } from "./threeKit";

const COLS = 64;
const ROWS = 48;
const HEIGHT = 1.8;

const VERT = /* glsl */ `
varying float vH;
void main(){
  vH = position.z;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}`;

const FRAG = /* glsl */ `
varying float vH;
void main(){
  vec3 green = vec3(0.16, 0.55, 0.32);
  vec3 gold = vec3(0.96, 0.77, 0.27);
  float t = clamp(vH * 1.4, 0.0, 1.0);
  gl_FragColor = vec4(mix(green, gold, t), 0.85);
}`;

/** Audio Terrain (3D) — a wireframe landscape that scrolls toward the camera;
 *  each frequency row extrudes into mountains as the spectrum streams in. */
export function AudioTerrain() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useThreeScene(canvasRef, ({ scene, camera }) => {
    camera.position.set(0, 2.4, 4.6);
    camera.lookAt(0, 0.2, -3);

    const geo = new THREE.PlaneGeometry(12, 9, COLS, ROWS);
    const mat = new THREE.ShaderMaterial({
      vertexShader: VERT,
      fragmentShader: FRAG,
      wireframe: true,
      transparent: true,
    });
    const mesh = new THREE.Mesh(geo, mat);
    // Tilt the mesh flat so local-z (the height we set per vertex) points up.
    mesh.rotation.x = -Math.PI / 2;
    scene.add(mesh);

    // heights[row][col] — newest row pushed at the far edge, scrolls to near.
    const heights: number[][] = Array.from({ length: ROWS + 1 }, () =>
      new Array(COLS + 1).fill(0),
    );
    const pos = geo.attributes.position as THREE.BufferAttribute;

    return {
      frame: (a) => {
        // Scroll: drop the near row, append a fresh far row from the spectrum.
        heights.shift();
        const spec = a.spectrum;
        const row = new Array(COLS + 1);
        for (let col = 0; col <= COLS; col++) {
          // Mirror across the centre so it's symmetric left↔right.
          const m = col <= COLS / 2 ? col : COLS - col;
          const idx = spec.length > 0 ? Math.floor((m / (COLS / 2)) * spec.length) : 0;
          row[col] = (spec.length > 0 ? spec[idx] ?? 0 : 0) * HEIGHT;
        }
        heights.push(row);

        for (let r = 0; r <= ROWS; r++) {
          const hr = heights[r]!;
          for (let col = 0; col <= COLS; col++) {
            // position is laid out row-major; after rotateX the height is z.
            pos.setZ(r * (COLS + 1) + col, hr[col] ?? 0);
          }
        }
        pos.needsUpdate = true;
      },
      dispose: () => {
        geo.dispose();
        mat.dispose();
      },
    };
  });

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
