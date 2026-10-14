"""Draw resampled medians from their exact rank distribution.

For the usual 30 pairs, one categorical draw replaces 30 draws and a sort.
This is the same nonparametric median bootstrap distribution; its seeded
random stream is deliberately versioned separately from the historical one.
"""
from bisect import bisect_right
from functools import lru_cache
import math


@lru_cache(maxsize=32)
def median_ranks(count):
    if type(count) is not int or not 1 <= count <= 128:
        raise ValueError('rank distribution requires 1..128 observations')
    middle=(count+1)//2
    def lower_cdf(probability):
        return math.fsum(math.comb(count, successes)*probability**successes*
                         (1-probability)**(count-successes)
                         for successes in range(middle,count+1))
    points=[]
    for low in range(count):
        lower_mass=lower_cdf((low+1)/count)-lower_cdf(low/count)
        if count%2:
            points.append(((low,low),max(0.0,lower_mass)))
            continue
        off_diagonal=[]
        for high in range(low+1,count):
            mass=math.comb(count,middle)*(
                ((low+1)/count)**middle-(low/count)**middle)*(
                ((count-high)/count)**(count-middle)-
                ((count-high-1)/count)**(count-middle))
            off_diagonal.append(((low,high),mass))
        diagonal=lower_mass-math.fsum(mass for _,mass in off_diagonal)
        if diagonal < -1e-12:
            raise ValueError('invalid bootstrap rank probability')
        points.append(((low,low),max(0.0,diagonal)))
        points.extend(off_diagonal)
    total=math.fsum(mass for _,mass in points)
    if not math.isclose(total,1.0,rel_tol=1e-11,abs_tol=1e-11):
        raise ValueError('bootstrap rank distribution is incomplete')
    cumulative=[];ranks=[];running=0.0
    for rank,mass in points:
        if mass:
            running+=mass/total;cumulative.append(min(running,1.0));ranks.append(rank)
    cumulative[-1]=1.0
    return tuple(cumulative),tuple(ranks)


def draw_median(ordered, rng):
    count=len(ordered)
    if count <= 128:
        cumulative,ranks=median_ranks(count)
        low,high=ranks[bisect_right(cumulative,rng.random())]
        return (ordered[low]+ordered[high])/2
    sample=sorted(ordered[rng.randrange(count)] for _ in range(count))
    return (sample[(count-1)//2]+sample[count//2])/2
