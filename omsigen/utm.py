"""WGS84/ETRS89 <-> UTM (Transversale Mercator, Krueger-Reihen nach Karney 2011, Genauigkeit << 1 mm).
ETRS89 und WGS84 unterscheiden sich um weniger als 1 m - fuer Gelaendehoehen unerheblich."""
import math

A = 6378137.0
F = 1 / 298.257223563
K0 = 0.9996
_n = F / (2 - F)
_A = A / (1 + _n) * (1 + _n ** 2 / 4 + _n ** 4 / 64)
_ALPHA = [_n / 2 - 2 * _n ** 2 / 3 + 5 * _n ** 3 / 16, 13 * _n ** 2 / 48 - 3 * _n ** 3 / 5, 61 * _n ** 3 / 240]
_BETA = [_n / 2 - 2 * _n ** 2 / 3 + 37 * _n ** 3 / 96, _n ** 2 / 48 + _n ** 3 / 15, 17 * _n ** 3 / 480]
_DELTA = [2 * _n - 2 * _n ** 2 / 3 - 2 * _n ** 3, 7 * _n ** 2 / 3 - 8 * _n ** 3 / 5, 56 * _n ** 3 / 15]


def to_utm(lat, lon, zone=32):
    """-> (Ost, Nord) in Metern (Nordhalbkugel)"""
    lon0 = math.radians(zone * 6 - 183)
    phi, lam = math.radians(lat), math.radians(lon) - lon0
    e = math.sqrt(F * (2 - F))
    t = math.sinh(math.atanh(math.sin(phi)) - e * math.atanh(e * math.sin(phi)))
    xi_ = math.atan2(t, math.cos(lam))
    eta_ = math.atanh(math.sin(lam) / math.sqrt(1 + t * t))
    xi, eta = xi_, eta_
    for j, a in enumerate(_ALPHA, 1):
        xi += a * math.sin(2 * j * xi_) * math.cosh(2 * j * eta_)
        eta += a * math.cos(2 * j * xi_) * math.sinh(2 * j * eta_)
    return 500000 + K0 * _A * eta, K0 * _A * xi


def from_utm(east, north, zone=32):
    """-> (lat, lon)"""
    xi = north / (K0 * _A)
    eta = (east - 500000) / (K0 * _A)
    xi_, eta_ = xi, eta
    for j, b in enumerate(_BETA, 1):
        xi_ -= b * math.sin(2 * j * xi) * math.cosh(2 * j * eta)
        eta_ -= b * math.cos(2 * j * xi) * math.sinh(2 * j * eta)
    chi = math.asin(math.sin(xi_) / math.cosh(eta_))
    phi = chi + sum(d * math.sin(2 * j * chi) for j, d in enumerate(_DELTA, 1))
    lam = math.atan2(math.sinh(eta_), math.cos(xi_))
    return math.degrees(phi), zone * 6 - 183 + math.degrees(lam)
