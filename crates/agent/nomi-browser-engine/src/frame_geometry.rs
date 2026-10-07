//! Browser-rendered content quads, not a second CSS transform implementation.
use nomifun_browser_platform::runtime::WorkspaceError;
use serde_json::Value;

#[derive(Clone, Copy)]
pub struct ContentQuad {
    points: [f64; 8],
    matrix: [f64; 8],
}
impl ContentQuad {
    pub fn read(value: &Value) -> Result<Self, WorkspaceError> {
        let values = value
            .as_array()
            .filter(|values| values.len() == 8)
            .ok_or(WorkspaceError::NotActionable)?;
        let mut q = [0.; 8];
        for (out, value) in q.iter_mut().zip(values) {
            *out = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or(WorkspaceError::NotActionable)?;
        }
        Self::new(q)
    }
    fn new(q: [f64; 8]) -> Result<Self, WorkspaceError> {
        if !q.iter().all(|value| value.is_finite()) {
            return Err(WorkspaceError::NotActionable);
        }
        let mut sign = 0.;
        for i in 0..4 {
            let a = i * 2;
            let b = ((i + 1) % 4) * 2;
            let c = ((i + 2) % 4) * 2;
            let cross =
                (q[b] - q[a]) * (q[c + 1] - q[b + 1]) - (q[b + 1] - q[a + 1]) * (q[c] - q[b]);
            if cross.abs() < 1e-6 || (sign != 0. && cross.signum() != sign) {
                return Err(WorkspaceError::NotActionable);
            }
            sign = cross.signum();
        }
        let (dx1, dx2, dx3) = (q[2] - q[4], q[6] - q[4], q[0] - q[2] + q[4] - q[6]);
        let (dy1, dy2, dy3) = (q[3] - q[5], q[7] - q[5], q[1] - q[3] + q[5] - q[7]);
        let (g, h) = if dx3.abs() + dy3.abs() < 1e-9 {
            (0., 0.)
        } else {
            let det = dx1 * dy2 - dx2 * dy1;
            if det.abs() < 1e-9 {
                return Err(WorkspaceError::NotActionable);
            }
            ((dx3 * dy2 - dx2 * dy3) / det, (dx1 * dy3 - dx3 * dy1) / det)
        };
        if [1. + g, 1. + h, 1. + g + h]
            .iter()
            .any(|w| !w.is_finite() || *w <= 1e-9)
        {
            return Err(WorkspaceError::NotActionable);
        }
        Ok(Self {
            points: q,
            matrix: [
                q[2] - q[0] + g * q[2],
                q[6] - q[0] + h * q[6],
                q[0],
                q[3] - q[1] + g * q[3],
                q[7] - q[1] + h * q[7],
                q[1],
                g,
                h,
            ],
        })
    }
    pub fn project(
        &self,
        point: (f64, f64),
        size: (f64, f64),
    ) -> Result<(f64, f64), WorkspaceError> {
        if !inside(point, size) {
            return Err(WorkspaceError::NotActionable);
        }
        let [a, b, c, d, e, f, g, h] = self.matrix;
        let (u, v) = (point.0 / size.0, point.1 / size.1);
        let w = g * u + h * v + 1.;
        finite(((a * u + b * v + c) / w, (d * u + e * v + f) / w))
    }
    pub fn unproject(
        &self,
        point: (f64, f64),
        size: (f64, f64),
    ) -> Result<(f64, f64), WorkspaceError> {
        let [a, b, c, d, e, f, g, h] = self.matrix;
        let (x, y) = point;
        let (aa, bb, dd, ee) = (a - x * g, b - x * h, d - y * g, e - y * h);
        let det = aa * ee - bb * dd;
        if det.abs() < 1e-9 {
            return Err(WorkspaceError::NotActionable);
        }
        let result = finite((
            ((x - c) * ee - bb * (y - f)) / det * size.0,
            (aa * (y - f) - (x - c) * dd) / det * size.1,
        ))?;
        if !inside(result, size) {
            return Err(WorkspaceError::NotActionable);
        }
        Ok(result)
    }
    pub fn unchanged(&self, other: &Self) -> bool {
        self.points
            .iter()
            .zip(other.points)
            .all(|(a, b)| (a - b).abs() <= 0.5)
    }
}
fn finite(point: (f64, f64)) -> Result<(f64, f64), WorkspaceError> {
    if point.0.is_finite() && point.1.is_finite() {
        Ok(point)
    } else {
        Err(WorkspaceError::NotActionable)
    }
}
fn inside(p: (f64, f64), s: (f64, f64)) -> bool {
    [p.0, p.1, s.0, s.1].iter().all(|x| x.is_finite())
        && s.0 > 0.
        && s.1 > 0.
        && p.0 >= 0.
        && p.1 >= 0.
        && p.0 < s.0
        && p.1 < s.1
}

// State and hit testing stay in the owning document; geometry comes from CDP.
pub const CHECK_FRAME_OWNER: &str = r#"async function(owner) {
    if (!owner?.isConnected || !['IFRAME','FRAME'].includes(owner.tagName)) return {error:'stale'};
    const state=await this.__nomiCheckStates(owner,['visible','stable']);
    return state ? {error:'not_actionable'} : {ready:true};
}"#;
pub const CHECK_FRAME_HIT: &str = r#"function(owner,x,y) {
    if (!owner?.isConnected) return {error:'stale'};
    const point={x,y};
    if (!(x>=0 && y>=0 && x<innerWidth && y<innerHeight) || this.expectHitTarget(point,owner)!=='done')
        return {error:'not_actionable'};
    return point;
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perspective_and_mirrored_quads_round_trip() {
        for q in [
            [20., 10., 160., 35., 130., 140., 5., 90.],
            [160., 10., 20., 35., 5., 140., 130., 90.],
            [0., 0., 200., 0., 200., 100., 0., 100.],
        ] {
            let quad = ContentQuad::new(q).unwrap();
            for point in [(0., 0.), (50., 25.), (190., 90.)] {
                let projected = quad.project(point, (200., 100.)).unwrap();
                let restored = quad.unproject(projected, (200., 100.)).unwrap();
                assert!((restored.0 - point.0).abs() < 1e-7 && (restored.1 - point.1).abs() < 1e-7);
            }
        }
    }
    #[test]
    fn perspective_midpoint_is_not_a_bilinear_average() {
        let quad=ContentQuad::new([0.,0.,100.,0.,75.,100.,25.,100.]).unwrap();
        let point=quad.project((50.,50.),(100.,100.)).unwrap();
        assert!((point.0-50.).abs()<1e-8);
        assert!((point.1-200./3.).abs()<1e-8);
    }
    #[test]
    fn unusable_quads_and_stale_geometry_do_not_produce_input_points() {
        for q in [
            [0.; 8],
            [0., 0., 100., 100., 100., 0., 0., 100.],
            [f64::NAN; 8],
        ] {
            assert!(ContentQuad::new(q).is_err());
        }
        let a = ContentQuad::new([0., 0., 100., 0., 100., 100., 0., 100.]).unwrap();
        let b = ContentQuad::new([1., 0., 101., 0., 101., 100., 1., 100.]).unwrap();
        assert!(!a.unchanged(&b));
        assert!(a.project((100., 50.), (100., 100.)).is_err());
    }
}
