//! A pure-Rust replacement for the only part of `ndarray-linalg` that
//! `speakrs` uses.
//!
//! `speakrs` reaches LAPACK in exactly three places, all in its PLDA setup
//! (`src/clustering/plda.rs`): two matrix inversions and one generalized
//! symmetric eigenproblem. Pulling that in through `ndarray-linalg` costs a
//! BLAS backend - a downloaded Intel MKL blob on x86_64, an OpenBLAS build
//! from source (and therefore a Fortran compiler) everywhere else. Neither is
//! acceptable for this app's Windows and macOS build pipelines.
//!
//! So the workspace patches `ndarray-linalg` with this crate, which implements
//! the same three operations on `nalgebra`: no C, no Fortran, no linker
//! surprises. Only the items `speakrs` imports are provided, and only for
//! `f64`. If `speakrs` ever reaches for more of `ndarray-linalg`, the build
//! fails loudly here rather than silently doing something different.
//!
//! Semantics deliberately match LAPACK's `dsygv`/`dgetri` as `ndarray-linalg`
//! exposes them: only the requested triangle of a symmetric input is read,
//! eigenvalues come back in ascending order, and generalized eigenvectors are
//! normalized so that `xᵀ B x = I`.

use nalgebra::{DMatrix, SymmetricEigen};
use ndarray::{Array1, Array2};

pub mod error {
    /// The failure modes the replaced operations can report.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LinalgError {
        /// The input was not square.
        NotSquare { rows: usize, cols: usize },
        /// Two inputs that had to agree on a dimension did not.
        Shape(&'static str),
        /// A matrix that had to be invertible was singular.
        Singular,
        /// The right-hand matrix of a generalized eigenproblem was not
        /// positive definite, so it has no Cholesky factor.
        NotPositiveDefinite,
    }

    impl std::fmt::Display for LinalgError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::NotSquare { rows, cols } => {
                    write!(formatter, "expected a square matrix, got {rows}x{cols}")
                }
                Self::Shape(detail) => write!(formatter, "incompatible shapes: {detail}"),
                Self::Singular => write!(formatter, "matrix is singular"),
                Self::NotPositiveDefinite => {
                    write!(formatter, "matrix is not positive definite")
                }
            }
        }
    }

    impl std::error::Error for LinalgError {}
}

pub use error::LinalgError;

pub type Result<T> = std::result::Result<T, LinalgError>;

/// Which triangle of a symmetric input carries the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UPLO {
    Upper,
    Lower,
}

/// Matrix inversion, mirroring `ndarray_linalg::Inverse`.
pub trait Inverse {
    type Output;

    fn inv(&self) -> Result<Self::Output>;
}

impl Inverse for Array2<f64> {
    type Output = Array2<f64>;

    fn inv(&self) -> Result<Array2<f64>> {
        let inverse = to_nalgebra(self)?
            .try_inverse()
            .ok_or(LinalgError::Singular)?;
        Ok(to_ndarray(&inverse))
    }
}

/// Symmetric eigendecomposition, mirroring `ndarray_linalg::Eigh`.
pub trait Eigh {
    type EigVal;
    type EigVec;

    fn eigh(&self, uplo: UPLO) -> Result<(Self::EigVal, Self::EigVec)>;
}

/// The generalized problem `A x = λ B x` for symmetric `A` and
/// symmetric positive-definite `B`.
///
/// `ndarray-linalg` returns the eigenvectors alongside the factorization of
/// `B`; `speakrs` discards that second matrix, but the shape of the return
/// type has to match, so the Cholesky factor is handed back in its place.
impl Eigh for (Array2<f64>, Array2<f64>) {
    type EigVal = Array1<f64>;
    type EigVec = (Array2<f64>, Array2<f64>);

    fn eigh(&self, uplo: UPLO) -> Result<(Self::EigVal, Self::EigVec)> {
        let left = symmetrize(to_nalgebra(&self.0)?, uplo);
        let right = symmetrize(to_nalgebra(&self.1)?, uplo);
        if left.nrows() != right.nrows() {
            return Err(LinalgError::Shape(
                "both operands of a generalized eigenproblem must have the same order",
            ));
        }

        // Reduce to a standard problem: with B = L Lᵀ, the eigenvalues of
        // C = L⁻¹ A L⁻ᵀ are those of the pair, and x = L⁻ᵀ y recovers the
        // generalized eigenvectors from C's orthonormal ones.
        let factor = right.cholesky().ok_or(LinalgError::NotPositiveDefinite)?.l();
        let half = factor
            .solve_lower_triangular(&left)
            .ok_or(LinalgError::Singular)?;
        // C is symmetric, so L⁻¹ (L⁻¹ A)ᵀ is already C - no transpose needed
        // afterwards, and both solves stay lower-triangular.
        let mut reduced = factor
            .solve_lower_triangular(&half.transpose())
            .ok_or(LinalgError::Singular)?;
        force_symmetry(&mut reduced);

        let eigen = SymmetricEigen::new(reduced);
        // Because y is orthonormal, x = L⁻ᵀ y satisfies xᵀ B x = I, which is
        // the normalization LAPACK's `dsygv` guarantees.
        let vectors = factor
            .transpose()
            .solve_upper_triangular(&eigen.eigenvectors)
            .ok_or(LinalgError::Singular)?;

        // LAPACK orders eigenvalues ascending; nalgebra leaves them unsorted.
        let mut order: Vec<usize> = (0..eigen.eigenvalues.len()).collect();
        order.sort_by(|&left_idx, &right_idx| {
            eigen.eigenvalues[left_idx]
                .partial_cmp(&eigen.eigenvalues[right_idx])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let values = Array1::from_iter(order.iter().map(|&index| eigen.eigenvalues[index]));
        let mut sorted = Array2::<f64>::zeros((vectors.nrows(), order.len()));
        for (target, &source) in order.iter().enumerate() {
            for row in 0..vectors.nrows() {
                sorted[[row, target]] = vectors[(row, source)];
            }
        }

        Ok((values, (sorted, to_ndarray(&factor))))
    }
}

fn to_nalgebra(values: &Array2<f64>) -> Result<DMatrix<f64>> {
    let (rows, cols) = values.dim();
    if rows != cols {
        return Err(LinalgError::NotSquare { rows, cols });
    }
    Ok(DMatrix::from_fn(rows, cols, |row, col| values[[row, col]]))
}

fn to_ndarray(values: &DMatrix<f64>) -> Array2<f64> {
    Array2::from_shape_fn((values.nrows(), values.ncols()), |(row, col)| {
        values[(row, col)]
    })
}

/// LAPACK reads only one triangle of a symmetric argument, so mirror that
/// triangle over the diagonal before handing the matrix to nalgebra, which
/// reads all of it.
fn symmetrize(mut matrix: DMatrix<f64>, uplo: UPLO) -> DMatrix<f64> {
    for row in 0..matrix.nrows() {
        for col in 0..row {
            match uplo {
                UPLO::Lower => matrix[(col, row)] = matrix[(row, col)],
                UPLO::Upper => matrix[(row, col)] = matrix[(col, row)],
            }
        }
    }
    matrix
}

/// Average away the asymmetry rounding leaves in the reduced matrix, so
/// `SymmetricEigen` is not fed a matrix it assumes is symmetric but is not.
fn force_symmetry(matrix: &mut DMatrix<f64>) {
    for row in 0..matrix.nrows() {
        for col in 0..row {
            let mean = 0.5 * (matrix[(row, col)] + matrix[(col, row)]);
            matrix[(row, col)] = mean;
            matrix[(col, row)] = mean;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{actual} is not close to {expected}"
        );
    }

    #[test]
    fn inverse_round_trips() {
        let matrix = array![[4.0, 7.0], [2.0, 6.0]];
        let inverse = matrix.inv().unwrap();
        let product = matrix.dot(&inverse);
        assert_close(product[[0, 0]], 1.0);
        assert_close(product[[1, 1]], 1.0);
        assert_close(product[[0, 1]], 0.0);
        assert_close(product[[1, 0]], 0.0);
    }

    #[test]
    fn singular_inverse_is_an_error() {
        let matrix = array![[1.0, 2.0], [2.0, 4.0]];
        assert_eq!(matrix.inv(), Err(LinalgError::Singular));
    }

    #[test]
    fn generalized_eigenproblem_matches_lapack_conventions() {
        let a = array![[3.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 3.0]];
        let b = array![[2.0, 0.5, 0.0], [0.5, 2.0, 0.5], [0.0, 0.5, 2.0]];

        let (values, (vectors, _)) = (a.clone(), b.clone()).eigh(UPLO::Lower).unwrap();

        // Ascending order, like LAPACK's dsygv.
        assert!(values[0] <= values[1] && values[1] <= values[2]);

        for index in 0..3 {
            let vector = vectors.column(index).to_owned();
            // A x = λ B x
            let left = a.dot(&vector);
            let right = b.dot(&vector).mapv(|value| value * values[index]);
            for row in 0..3 {
                assert_close(left[row], right[row]);
            }
        }

        // B-orthonormal: xᵀ B x = I.
        let gram = vectors.t().dot(&b).dot(&vectors);
        for row in 0..3 {
            for col in 0..3 {
                assert_close(gram[[row, col]], if row == col { 1.0 } else { 0.0 });
            }
        }
    }

    #[test]
    fn only_the_requested_triangle_is_read() {
        // The upper triangles disagree; reading the lower one must make both
        // calls agree, the way LAPACK with UPLO='L' would.
        let clean = array![[2.0, 1.0], [1.0, 2.0]];
        let dirty = array![[2.0, 99.0], [1.0, 2.0]];
        let identity = array![[1.0, 0.0], [0.0, 1.0]];

        let (from_clean, _) = (clean, identity.clone()).eigh(UPLO::Lower).unwrap();
        let (from_dirty, _) = (dirty, identity).eigh(UPLO::Lower).unwrap();

        for index in 0..2 {
            assert_close(from_clean[index], from_dirty[index]);
        }
    }
}
