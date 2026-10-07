import { useRef, useState, useEffect, useLayoutEffect, useCallback, RefObject } from "react";
import { Virtualizer } from "@tanstack/react-virtual";

interface UseAutoScrollProps {
    scrollRef: RefObject<HTMLDivElement | null>;
    contentRef?: RefObject<HTMLDivElement | null>;
    segments: any[];
    isRecording: boolean;
    isPaused: boolean;
    activeSegmentId?: string;
    virtualizer?: Virtualizer<HTMLDivElement, Element>;
    virtualizationThreshold?: number;
    disableAutoScroll?: boolean;
}

// Allow a small rounding/touch tolerance without pulling readers back down.
const SCROLL_THRESHOLD = 24;

export function useAutoScroll({
    scrollRef,
    contentRef,
    segments,
    isRecording,
    isPaused,
    activeSegmentId,
    virtualizer,
    virtualizationThreshold = 10,
    disableAutoScroll = false,
}: UseAutoScrollProps) {
    const [autoScroll, setAutoScrollState] = useState(true);
    const autoScrollRef = useRef(true);
    const lastScrollTopRef = useRef(0);
    const frameRef = useRef<number | null>(null);
    const followingEnabled = isRecording && !isPaused && !disableAutoScroll;

    const setAutoScroll = useCallback((value: boolean) => {
        autoScrollRef.current = value;
        setAutoScrollState(value);
    }, []);

    const followBottom = useCallback(() => {
        const container = scrollRef.current;
        if (!container) return;
        // Include the listening indicator and measured virtual rows.
        container.scrollTop = container.scrollHeight;
        lastScrollTopRef.current = container.scrollTop;
    }, [scrollRef]);

    const scrollToBottom = useCallback(() => {
        setAutoScroll(true);
        followBottom();
    }, [setAutoScroll, followBottom]);

    useEffect(() => {
        if (disableAutoScroll) return;
        const container = scrollRef.current;
        if (!container) return;
        lastScrollTopRef.current = container.scrollTop;
        const handleScroll = () => {
            const nearBottom = container.scrollHeight - container.scrollTop - container.clientHeight <= SCROLL_THRESHOLD;
            if (nearBottom) {
                setAutoScroll(true);
            } else if (container.scrollTop < lastScrollTopRef.current) {
                // Update immediately so incoming phrases cannot beat a debounce.
                setAutoScroll(false);
            }
            lastScrollTopRef.current = container.scrollTop;
        };
        container.addEventListener("scroll", handleScroll, { passive: true });
        return () => container.removeEventListener("scroll", handleScroll);
    }, [scrollRef, setAutoScroll, disableAutoScroll]);

    // Preserve the position BEFORE content grows: a long new phrase must not
    // be mistaken for the user scrolling away from the bottom.
    useLayoutEffect(() => {
        if (segments.length === 0) setAutoScroll(true);
        if (followingEnabled && autoScrollRef.current) followBottom();
    }, [segments, followingEnabled, followBottom, setAutoScroll]);

    // Streaming text and measured virtual rows can grow without new segments.
    useEffect(() => {
        const container = scrollRef.current;
        const content = contentRef?.current;
        if (!followingEnabled || !container || !content) return;
        const observer = new ResizeObserver(() => {
            if (!autoScrollRef.current || frameRef.current !== null) return;
            frameRef.current = requestAnimationFrame(() => {
                frameRef.current = null;
                if (autoScrollRef.current) followBottom();
            });
        });
        observer.observe(content);
        observer.observe(container);
        return () => {
            observer.disconnect();
            if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
            frameRef.current = null;
        };
    }, [followingEnabled, contentRef, scrollRef, followBottom]);

    useEffect(() => {
        if (!activeSegmentId) return;
        setAutoScroll(false);
        if (virtualizer && segments.length >= virtualizationThreshold) {
            const index = segments.findIndex(segment => segment.id === activeSegmentId);
            if (index >= 0) virtualizer.scrollToIndex(index, { align: "center" });
        } else {
            document.getElementById(`segment-${activeSegmentId}`)?.scrollIntoView({ block: "center" });
        }
    }, [activeSegmentId, segments, virtualizer, virtualizationThreshold, setAutoScroll]);

    return { autoScroll, setAutoScroll, scrollToBottom };
}
