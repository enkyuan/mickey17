import { animate } from 'motion';
import { useEffect, type RefObject } from 'react';
import { useMotionPreference } from '@/components/providers/motion-preference';

type RevealOptions = {
  effect: 'fade' | 'rise' | 'scale';
  distance: number;
  duration: number;
  stagger: number;
  threshold: number;
};

/** Progressive enhancement: SSR stays visible, and reduced motion cancels all effects. */
export function useScrollReveal(root: RefObject<HTMLElement | null>, targetClass: string, options: RevealOptions) {
  const { reducedMotion } = useMotionPreference();
  const { effect, distance, duration, stagger, threshold } = options;
  useEffect(() => {
    const container = root.current;
    if (!container || reducedMotion) return;
    const targets = Array.from(container.querySelectorAll<HTMLElement>(`.${CSS.escape(targetClass)}`));
    if (container.classList.contains(targetClass)) targets.unshift(container);
    const animations = new Map<Element, ReturnType<typeof animate>>();
    for (const [index, target] of targets.entries()) {
      const transform = effect === 'rise' ? `translateY(${distance}px)` : effect === 'scale' ? `scale(${1 + distance / 100})` : 'none';
      const animation = animate(target, { opacity: [0, 1], transform: [transform, 'none'] }, {
        duration: duration / 1000, delay: Math.min(index, 20) * stagger / 1000,
        ease: [0.43, 0.195, 0.02, 1], autoplay: false,
      });
      animation.pause();
      animations.set(target, animation);
    }
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        animations.get(entry.target)?.play();
        observer.unobserve(entry.target);
      }
    }, { threshold });
    for (const target of targets) observer.observe(target);
    return () => {
      observer.disconnect();
      for (const animation of animations.values()) animation.cancel();
    };
  }, [root, targetClass, effect, distance, duration, stagger, threshold, reducedMotion]);
}
