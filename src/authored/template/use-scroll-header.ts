import { frame, cancelFrame } from 'motion';
import { useEffect, type RefObject } from 'react';
import { useMotionPreference } from '@/components/providers/motion-preference';

/** Accumulate scroll deltas so reversing direction reveals the header immediately. */
export function useScrollHeader(root: RefObject<HTMLElement | null>, minimumWidth: number) {
  const { reducedMotion } = useMotionPreference();
  useEffect(() => {
    const header = root.current;
    if (!header || reducedMotion) return;
    const pointer = window.matchMedia(`(min-width: ${minimumWidth}px) and (hover: hover) and (pointer: fine)`);
    const previousTransform = header.style.transform;
    let previousScroll = window.scrollY;
    let offset = 0;
    let maximumOffset = 0;
    let scheduled = false;
    function update() {
      scheduled = false;
      const position = Math.max(0, window.scrollY);
      const interacting = header!.contains(document.activeElement) || !!header!.querySelector('[aria-expanded="true"]');
      if (!pointer.matches || position === 0 || interacting) offset = 0;
      else offset = Math.min(maximumOffset, Math.max(0, offset + position - previousScroll));
      header!.style.transform = `translateY(${-offset}px)`;
      previousScroll = position;
    }
    function schedule() { if (!scheduled) { scheduled = true; frame.render(update); } }
    function measure() {
      maximumOffset = header!.offsetHeight + Math.max(0, Number.parseFloat(getComputedStyle(header!).top) || 0);
      offset = Math.min(offset, maximumOffset);
      schedule();
    }
    const resize = new ResizeObserver(measure);
    resize.observe(header);
    measure();
    window.addEventListener('scroll', schedule, { passive: true });
    window.addEventListener('resize', measure);
    pointer.addEventListener('change', measure);
    header.addEventListener('focusin', schedule);
    return () => {
      window.removeEventListener('scroll', schedule);
      window.removeEventListener('resize', measure);
      pointer.removeEventListener('change', measure);
      header.removeEventListener('focusin', schedule);
      resize.disconnect();
      cancelFrame(update);
      header.style.transform = previousTransform;
    };
  }, [root, minimumWidth, reducedMotion]);
}
