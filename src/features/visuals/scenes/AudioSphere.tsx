import { useEffect, useRef } from "react";
import * as THREE from "three";
import { MAX_DPR, useAudioData } from "./sceneKit";

// Ashima/Gustavson 3D simplex noise — drives the surface displacement.
const SNOISE = /* glsl */ `
vec3 mod289(vec3 x){return x - floor(x*(1.0/289.0))*289.0;}
vec4 mod289(vec4 x){return x - floor(x*(1.0/289.0))*289.0;}
vec4 permute(vec4 x){return mod289(((x*34.0)+1.0)*x);}
vec4 taylorInvSqrt(vec4 r){return 1.79284291400159 - 0.85373472095314 * r;}
float snoise(vec3 v){
  const vec2 C = vec2(1.0/6.0, 1.0/3.0);
  const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
  vec3 i = floor(v + dot(v, C.yyy));
  vec3 x0 = v - i + dot(i, C.xxx);
  vec3 g = step(x0.yzx, x0.xyz);
  vec3 l = 1.0 - g;
  vec3 i1 = min(g.xyz, l.zxy);
  vec3 i2 = max(g.xyz, l.zxy);
  vec3 x1 = x0 - i1 + C.xxx;
  vec3 x2 = x0 - i2 + C.yyy;
  vec3 x3 = x0 - D.yyy;
  i = mod289(i);
  vec4 p = permute(permute(permute(
    i.z + vec4(0.0, i1.z, i2.z, 1.0))
    + i.y + vec4(0.0, i1.y, i2.y, 1.0))
    + i.x + vec4(0.0, i1.x, i2.x, 1.0));
  float n_ = 0.142857142857;
  vec3 ns = n_ * D.wyz - D.xzx;
  vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
  vec4 x_ = floor(j * ns.z);
  vec4 y_ = floor(j - 7.0 * x_);
  vec4 x = x_ * ns.x + ns.yyyy;
  vec4 y = y_ * ns.x + ns.yyyy;
  vec4 h = 1.0 - abs(x) - abs(y);
  vec4 b0 = vec4(x.xy, y.xy);
  vec4 b1 = vec4(x.zw, y.zw);
  vec4 s0 = floor(b0)*2.0 + 1.0;
  vec4 s1 = floor(b1)*2.0 + 1.0;
  vec4 sh = -step(h, vec4(0.0));
  vec4 a0 = b0.xzyw + s0.xzyw*sh.xxyy;
  vec4 a1 = b1.xzyw + s1.xzyw*sh.zzww;
  vec3 p0 = vec3(a0.xy, h.x);
  vec3 p1 = vec3(a0.zw, h.y);
  vec3 p2 = vec3(a1.xy, h.z);
  vec3 p3 = vec3(a1.zw, h.w);
  vec4 norm = taylorInvSqrt(vec4(dot(p0,p0), dot(p1,p1), dot(p2,p2), dot(p3,p3)));
  p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
  vec4 m = max(0.6 - vec4(dot(x0,x0), dot(x1,x1), dot(x2,x2), dot(x3,x3)), 0.0);
  m = m * m;
  return 42.0 * dot(m*m, vec4(dot(p0,x0), dot(p1,x1), dot(p2,x2), dot(p3,x3)));
}`;

const VERT = /* glsl */ `
uniform float uTime; uniform float uBass; uniform float uBeat;
varying float vDisp; varying vec3 vNormal; varying vec3 vView;
${SNOISE}
void main(){
  vNormal = normalize(normalMatrix * normal);
  float n = snoise(normal * 1.8 + uTime * 0.25);
  float amp = 0.16 + uBass * 0.85 + uBeat * 0.45;
  float disp = n * amp;
  vDisp = disp;
  vec3 pos = position + normal * disp;
  vec4 mv = modelViewMatrix * vec4(pos, 1.0);
  vView = normalize(-mv.xyz);
  gl_Position = projectionMatrix * mv;
}`;

const FRAG = /* glsl */ `
uniform float uTreble; uniform float uBeat;
varying float vDisp; varying vec3 vNormal; varying vec3 vView;
void main(){
  vec3 gold = vec3(0.96, 0.77, 0.27);
  vec3 green = vec3(0.29, 0.87, 0.50);
  float t = clamp(vDisp * 1.5 + 0.5, 0.0, 1.0);
  vec3 base = mix(green, gold, t) * (0.5 + 0.5 * t);
  float fres = pow(1.0 - max(dot(normalize(vNormal), normalize(vView)), 0.0), 2.0);
  vec3 col = base + fres * (0.5 + uBeat) * gold + uTreble * 0.25;
  gl_FragColor = vec4(col, 1.0);
}`;

const damp = (cur: number, target: number, k: number) => cur + (target - cur) * k;

/**
 * Audio Sphere (3D) — an icosphere whose surface is displaced by simplex noise
 * driven by bass + beat, with a gold→green gradient and a fresnel rim glow.
 * Three.js, DPR-capped, fully disposed on unmount. Lazy-loaded (carries three).
 */
export function AudioSphere() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { sample } = useAudioData();

  useEffect(() => {
    const canvas = canvasRef.current;
    const parent = canvas?.parentElement;
    if (!canvas || !parent) return;

    const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(MAX_DPR, window.devicePixelRatio || 1));

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 100);
    camera.position.z = 3.2;

    const uniforms = {
      uTime: { value: 0 },
      uBass: { value: 0 },
      uMid: { value: 0 },
      uTreble: { value: 0 },
      uBeat: { value: 0 },
    };
    const geo = new THREE.IcosahedronGeometry(1.1, 5);
    const mat = new THREE.ShaderMaterial({
      uniforms,
      vertexShader: VERT,
      fragmentShader: FRAG,
    });
    const mesh = new THREE.Mesh(geo, mat);
    scene.add(mesh);

    const resize = () => {
      const w = Math.max(1, parent.clientWidth);
      const h = Math.max(1, parent.clientHeight);
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(parent);

    let raf = 0;
    let last = performance.now();
    const draw = () => {
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const a = sample(dt);
      uniforms.uTime.value += dt;
      uniforms.uBass.value = damp(uniforms.uBass.value, a.bass, 0.3);
      uniforms.uMid.value = damp(uniforms.uMid.value, a.mid, 0.3);
      uniforms.uTreble.value = damp(uniforms.uTreble.value, a.treble, 0.3);
      uniforms.uBeat.value = damp(uniforms.uBeat.value, a.beat, 0.5);
      mesh.rotation.y += dt * 0.15;
      mesh.rotation.x += dt * 0.05;
      mesh.scale.setScalar(1 + a.beat * 0.08);
      renderer.render(scene, camera);
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      geo.dispose();
      mat.dispose();
      renderer.dispose();
    };
  }, [sample]);

  return <canvas ref={canvasRef} className="block size-full" aria-hidden="true" />;
}
