import { useRef } from "react";
import { GOLD, GREEN, THREE, useThreeScene } from "./threeKit";

const COUNT = 9000;
const BRANCHES = 4;
const RADIUS = 4;

const VERT = /* glsl */ `
uniform float uSize; uniform float uBeat; uniform float uBass;
attribute vec3 aColor; attribute float aScale;
varying vec3 vColor;
void main(){
  vColor = aColor;
  vec3 pos = position * (1.0 + uBeat * 0.18 + uBass * 0.12);
  vec4 mv = modelViewMatrix * vec4(pos, 1.0);
  gl_Position = projectionMatrix * mv;
  gl_PointSize = uSize * aScale * (1.0 + uBeat * 0.8) * (1.0 / -mv.z);
}`;

const FRAG = /* glsl */ `
varying vec3 vColor;
void main(){
  float d = distance(gl_PointCoord, vec2(0.5));
  if (d > 0.5) discard;
  gl_FragColor = vec4(vColor, smoothstep(0.5, 0.0, d));
}`;

/** Particle Galaxy (3D) — a spiral of GPU points (gold core → green rim) that
 *  rotates, expands on bass/beat, and glows via additive blending. */
export function ParticleGalaxy() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useThreeScene(canvasRef, ({ scene, camera }) => {
    camera.position.set(0, 1.3, 6);
    camera.lookAt(0, 0, 0);

    const positions = new Float32Array(COUNT * 3);
    const colors = new Float32Array(COUNT * 3);
    const scales = new Float32Array(COUNT);
    const c = new THREE.Color();
    for (let i = 0; i < COUNT; i++) {
      const r = Math.pow(Math.random(), 1.6) * RADIUS;
      const branch = ((i % BRANCHES) / BRANCHES) * Math.PI * 2;
      const spin = r * 1.1;
      positions[i * 3] = Math.cos(branch + spin) * r + (Math.random() - 0.5) * 0.5 * r;
      positions[i * 3 + 1] = (Math.random() - 0.5) * 0.4 * r;
      positions[i * 3 + 2] = Math.sin(branch + spin) * r + (Math.random() - 0.5) * 0.5 * r;
      c.copy(GOLD).lerp(GREEN, Math.min(1, r / RADIUS));
      colors[i * 3] = c.r;
      colors[i * 3 + 1] = c.g;
      colors[i * 3 + 2] = c.b;
      scales[i] = 0.5 + Math.random() * 1.6;
    }

    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geo.setAttribute("aColor", new THREE.BufferAttribute(colors, 3));
    geo.setAttribute("aScale", new THREE.BufferAttribute(scales, 1));

    const uniforms = { uSize: { value: 9 }, uBeat: { value: 0 }, uBass: { value: 0 } };
    const mat = new THREE.ShaderMaterial({
      uniforms,
      vertexShader: VERT,
      fragmentShader: FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    const points = new THREE.Points(geo, mat);
    scene.add(points);

    return {
      frame: (a, dt) => {
        uniforms.uBeat.value += (a.beat - uniforms.uBeat.value) * 0.4;
        uniforms.uBass.value += (a.bass - uniforms.uBass.value) * 0.3;
        points.rotation.y += dt * (0.06 + a.level * 0.25);
      },
      dispose: () => {
        geo.dispose();
        mat.dispose();
      },
    };
  });

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
