export interface Point {
    x: number;
    y: number;
}

export interface Boundaries {
    field: Point[];
    left_end_zone: Point[];
    right_end_zone: Point[];
}

export interface ROI {
    x: number;
    y: number;
    width: number;
    height: number;
    x_normalized: number;
    y_normalized: number;
    width_normalized: number;
    height_normalized: number;
    crop_scale: number;
    source: string;
}

export interface FieldBoundariesConfig extends Boundaries {
    roi: ROI;
}

export const BOUNDARY_CHOICES = [
    { key: 'field', label: 'Field', color: '#ff4444' },
    { key: 'left_end_zone', label: 'Left End Zone', color: '#44ff44' },
    { key: 'right_end_zone', label: 'Right End Zone', color: '#4444ff' },
] as const;

export type BoundaryKey = (typeof BOUNDARY_CHOICES)[number]['key'];

export function computeROI(
    boundaries: Boundaries,
    imageWidth: number,
    imageHeight: number
): ROI | null {
    const allPoints = Object.values(boundaries).flat();
    if (allPoints.length === 0) return null;

    const minX = Math.min(...allPoints.map((p) => p.x));
    const maxX = Math.max(...allPoints.map((p) => p.x));
    const minY = Math.min(...allPoints.map((p) => p.y));
    const maxY = Math.max(...allPoints.map((p) => p.y));

    const widthSpan = Math.max(maxX - minX, 1.0);
    const heightSpan = Math.max(maxY - minY, 1.0);
    const padX = Math.max(widthSpan * 0.05, 1.0);
    const padY = Math.max(heightSpan * 0.05, 1.0);

    const roiX = Math.max(0, Math.floor(minX - padX));
    const roiY = Math.max(0, Math.floor(minY - padY));
    const roiWidth = Math.min(imageWidth - roiX, Math.ceil(maxX + padX) - roiX);
    const roiHeight = Math.min(imageHeight - roiY, Math.ceil(maxY + padY) - roiY);

    return {
        x: roiX,
        y: roiY,
        width: roiWidth,
        height: roiHeight,
        x_normalized: roiX / imageWidth,
        y_normalized: roiY / imageHeight,
        width_normalized: roiWidth / imageWidth,
        height_normalized: roiHeight / imageHeight,
        crop_scale: 1.0,
        source: 'boundary_editor',
    };
}

export function normalizeBoundaries(
    boundaries: Boundaries,
    roi: ROI
): Boundaries {
    const normalized: Partial<Boundaries> = {};
    for (const [key, points] of Object.entries(boundaries)) {
        normalized[key as keyof Boundaries] = points.map((p: Point) => ({
            x: Math.max(0, Math.min(1, (p.x - roi.x) / roi.width)),
            y: Math.max(0, Math.min(1, (p.y - roi.y) / roi.height)),
        }));
    }
    return normalized as Boundaries;
}
