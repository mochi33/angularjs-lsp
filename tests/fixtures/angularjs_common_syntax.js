// ============================================================
// AngularJS 一般的な構文パターン テストファイル
// LSPの対応状況を調査するための網羅的なテストケース
// ============================================================

// ============================
// 1. Module定義パターン
// ============================

// 1.1 基本的なモジュール定義（依存あり）
angular.module('commonApp', ['ngRoute', 'ngAnimate', 'ngResource']);

// 1.2 依存なしのモジュール定義
angular.module('simpleApp', []);

// 1.3 既存モジュール参照（定義済みモジュールへのアクセス）
angular.module('commonApp');

// ============================
// 2. Controller定義パターン
// ============================

// 2.1 DI配列記法（最も一般的）
angular.module('commonApp').controller('ArrayDIController', ['$scope', '$http', 'UserService', function($scope, $http, UserService) {
    $scope.users = [];
    $scope.loading = false;

    $scope.loadUsers = function() {
        $scope.loading = true;
        UserService.getAll().then(function(data) {
            $scope.users = data;
            $scope.loading = false;
        });
    };

    $scope.deleteUser = function(userId) {
        $http.delete('/api/users/' + userId);
    };
}]);

// 2.2 $inject パターン
function InjectStyleController($scope, $timeout, DataService) {
    $scope.message = 'Hello';
    $scope.counter = 0;

    $scope.increment = function() {
        $scope.counter++;
    };

    $scope.delayedMessage = function(msg) {
        $timeout(function() {
            $scope.message = msg;
        }, 1000);
    };
}
InjectStyleController.$inject = ['$scope', '$timeout', 'DataService'];
angular.module('commonApp').controller('InjectStyleController', InjectStyleController);

// 2.3 関数参照パターン（$injectなし）
function SimpleFuncController($scope) {
    $scope.title = 'Simple Function Controller';
}
angular.module('commonApp').controller('SimpleFuncController', SimpleFuncController);

// 2.4 ES6 class パターン
class ClassController {
    constructor($scope, $http) {
        $scope.items = [];
        $scope.fetchItems = function() {
            $http.get('/api/items').then(function(response) {
                $scope.items = response.data;
            });
        };
    }

    // class メソッド (controller as で使用)
    refresh() {
        console.log('refreshing');
    }
}
ClassController.$inject = ['$scope', '$http'];
angular.module('commonApp').controller('ClassController', ClassController);

// 2.5 インラインclass式
angular.module('commonApp').controller('InlineClassController', class {
    constructor($scope) {
        $scope.inlineMessage = 'From inline class';
    }
});

// 2.6 DI配列内class式
angular.module('commonApp').controller('ArrayClassController', ['$scope', '$log', class {
    constructor($scope, $log) {
        $scope.logged = true;
        $log.info('ArrayClassController initialized');
    }
}]);

// 2.7 controller as パターン（thisの使用）
angular.module('commonApp').controller('ControllerAsCtrl', ['$http', function($http) {
    var vm = this;
    vm.title = 'Controller As Pattern';
    vm.items = [];

    vm.loadItems = function() {
        $http.get('/api/items').then(function(response) {
            vm.items = response.data;
        });
    };
}]);

// 2.8 arrow function でのDI（非推奨だが使われることがある）
angular.module('commonApp').controller('ArrowController', ['$scope', ($scope) => {
    $scope.arrowMessage = 'From arrow function';
}]);

// ============================
// 3. Service定義パターン
// ============================

// 3.1 基本的なservice定義（DI配列記法）
angular.module('commonApp').service('UserService', ['$http', '$q', function($http, $q) {
    this.getAll = function() {
        return $http.get('/api/users').then(function(res) { return res.data; });
    };

    this.getById = function(id) {
        return $http.get('/api/users/' + id).then(function(res) { return res.data; });
    };

    this.create = function(user) {
        return $http.post('/api/users', user);
    };

    this.update = function(id, user) {
        return $http.put('/api/users/' + id, user);
    };

    this.delete = function(id) {
        return $http.delete('/api/users/' + id);
    };
}]);

// 3.2 class-based service
class DataService {
    constructor($http, $cacheFactory) {
        this.http = $http;
        this.cache = $cacheFactory('dataCache');
    }

    getData(key) {
        var cached = this.cache.get(key);
        if (cached) return cached;
        return this.http.get('/api/data/' + key);
    }

    clearCache() {
        this.cache.removeAll();
    }
}
DataService.$inject = ['$http', '$cacheFactory'];
angular.module('commonApp').service('DataService', DataService);

// 3.3 service without DI array (implicit injection - dangerous for minification)
angular.module('commonApp').service('SimpleService', function($http) {
    this.fetch = function() {
        return $http.get('/api/simple');
    };
});

// ============================
// 4. Factory定義パターン
// ============================

// 4.1 基本的なfactory定義
angular.module('commonApp').factory('AuthService', ['$http', '$q', '$window', function($http, $q, $window) {
    var service = {};

    service.login = function(credentials) {
        return $http.post('/api/auth/login', credentials).then(function(response) {
            $window.localStorage.setItem('token', response.data.token);
            return response.data;
        });
    };

    service.logout = function() {
        $window.localStorage.removeItem('token');
    };

    service.isAuthenticated = function() {
        return !!$window.localStorage.getItem('token');
    };

    service.getToken = function() {
        return $window.localStorage.getItem('token');
    };

    return service;
}]);

// 4.2 Revealing Module Pattern
angular.module('commonApp').factory('UtilService', [function() {
    function formatDate(date) {
        return date.toISOString();
    }

    function capitalize(str) {
        return str.charAt(0).toUpperCase() + str.slice(1);
    }

    return {
        formatDate: formatDate,
        capitalize: capitalize
    };
}]);

// 4.3 class-based factory
class NotificationFactory {
    constructor($timeout) {
        this.timeout = $timeout;
        this.notifications = [];
    }

    add(message, type) {
        var notification = { message: message, type: type || 'info' };
        this.notifications.push(notification);
    }

    remove(index) {
        this.notifications.splice(index, 1);
    }
}
NotificationFactory.$inject = ['$timeout'];
angular.module('commonApp').factory('NotificationService', NotificationFactory);

// ============================
// 5. Directive定義パターン
// ============================

// 5.1 基本的なdirective（restrict: 'E' - Element）
angular.module('commonApp').directive('userCard', [function() {
    return {
        restrict: 'E',
        scope: {
            user: '=',
            onSelect: '&'
        },
        templateUrl: 'templates/user-card.html',
        link: function(scope, element, attrs) {
            scope.select = function() {
                scope.onSelect({ user: scope.user });
            };
        }
    };
}]);

// 5.2 Attribute directive
angular.module('commonApp').directive('myHighlight', [function() {
    return {
        restrict: 'A',
        link: function(scope, element, attrs) {
            element.on('mouseenter', function() {
                element.css('background-color', attrs.myHighlight || 'yellow');
            });
            element.on('mouseleave', function() {
                element.css('background-color', '');
            });
        }
    };
}]);

// 5.3 Directive with controller
angular.module('commonApp').directive('tabPanel', ['$compile', function($compile) {
    return {
        restrict: 'E',
        transclude: true,
        scope: {
            tabs: '='
        },
        controller: ['$scope', function($scope) {
            $scope.activeTab = 0;
            $scope.selectTab = function(index) {
                $scope.activeTab = index;
            };
        }],
        templateUrl: 'templates/tab-panel.html'
    };
}]);

// 5.4 Directive returning function (link function only)
angular.module('commonApp').directive('autoFocus', ['$timeout', function($timeout) {
    return function(scope, element) {
        $timeout(function() {
            element[0].focus();
        });
    };
}]);

// 5.5 Directive with require (parent directive)
angular.module('commonApp').directive('tabItem', [function() {
    return {
        restrict: 'E',
        require: '^tabPanel',
        scope: {
            title: '@'
        },
        link: function(scope, element, attrs, tabPanelCtrl) {
            // interact with parent directive controller
        }
    };
}]);

// 5.6 Directive with compile function
angular.module('commonApp').directive('repeatDirective', [function() {
    return {
        restrict: 'A',
        compile: function(tElement, tAttrs) {
            // compile phase
            return {
                pre: function(scope, element, attrs) {
                    // pre-link
                },
                post: function(scope, element, attrs) {
                    // post-link
                }
            };
        }
    };
}]);

// ============================
// 6. Component定義パターン (AngularJS 1.5+)
// ============================

// 6.1 基本的なcomponent定義
angular.module('commonApp').component('heroDetail', {
    templateUrl: 'templates/hero-detail.html',
    controller: 'HeroDetailController',
    controllerAs: 'vm',
    bindings: {
        hero: '<',
        onDelete: '&',
        onUpdate: '&'
    }
});

// 6.2 インラインcontroller付きcomponent
angular.module('commonApp').component('heroList', {
    templateUrl: 'templates/hero-list.html',
    controller: ['HeroService', function(HeroService) {
        var ctrl = this;

        ctrl.$onInit = function() {
            HeroService.getAll().then(function(heroes) {
                ctrl.heroes = heroes;
            });
        };

        ctrl.$onChanges = function(changes) {
            if (changes.filter) {
                ctrl.applyFilter();
            }
        };
    }],
    bindings: {
        filter: '<'
    }
});

// 6.3 ライフサイクルフック付きcomponent
angular.module('commonApp').component('lifecycleDemo', {
    template: '<div>{{ $ctrl.status }}</div>',
    controller: function() {
        var ctrl = this;
        ctrl.status = 'created';

        ctrl.$onInit = function() {
            ctrl.status = 'initialized';
        };

        ctrl.$onDestroy = function() {
            // cleanup
        };

        ctrl.$doCheck = function() {
            // custom change detection
        };

        ctrl.$postLink = function() {
            // DOM is ready
        };
    }
});

// ============================
// 7. Provider定義パターン
// ============================

// 7.1 基本的なprovider定義
angular.module('commonApp').provider('apiConfig', function() {
    var baseUrl = '/api';
    var version = 'v1';

    this.setBaseUrl = function(url) {
        baseUrl = url;
    };

    this.setVersion = function(v) {
        version = v;
    };

    this.$get = ['$http', function($http) {
        return {
            getUrl: function(path) {
                return baseUrl + '/' + version + path;
            },
            request: function(path) {
                return $http.get(baseUrl + '/' + version + path);
            }
        };
    }];
});

// ============================
// 8. Filter定義パターン
// ============================

// 8.1 基本的なfilter定義
angular.module('commonApp').filter('capitalize', [function() {
    return function(input) {
        if (!input) return '';
        return input.charAt(0).toUpperCase() + input.slice(1);
    };
}]);

// 8.2 パラメータ付きfilter
angular.module('commonApp').filter('truncate', [function() {
    return function(input, length, suffix) {
        if (!input) return '';
        length = length || 100;
        suffix = suffix || '...';
        if (input.length <= length) return input;
        return input.substring(0, length) + suffix;
    };
}]);

// 8.3 service依存のfilter
angular.module('commonApp').filter('currency', ['$locale', function($locale) {
    return function(amount, symbol) {
        symbol = symbol || $locale.NUMBER_FORMATS.CURRENCY_SYM;
        return symbol + amount.toFixed(2);
    };
}]);

// ============================
// 9. Constant/Value定義パターン
// ============================

// 9.1 Constant定義
angular.module('commonApp').constant('API_URL', 'https://api.example.com');
angular.module('commonApp').constant('APP_CONFIG', {
    debug: false,
    version: '2.0.0',
    maxRetries: 3
});

// 9.2 Value定義
angular.module('commonApp').value('appVersion', '1.0.0');
angular.module('commonApp').value('defaultSettings', {
    theme: 'light',
    language: 'ja',
    pageSize: 20
});

// ============================
// 10. .config() / .run() パターン
// ============================

// 10.1 .config() ブロック
angular.module('commonApp').config(['$routeProvider', '$locationProvider', function($routeProvider, $locationProvider) {
    $locationProvider.html5Mode(true);

    $routeProvider
        .when('/users', {
            templateUrl: 'views/users.html',
            controller: 'ArrayDIController'
        })
        .when('/users/:id', {
            templateUrl: 'views/user-detail.html',
            controller: 'UserDetailController'
        })
        .otherwise({
            redirectTo: '/users'
        });
}]);

// 10.2 .run() ブロック（$rootScope使用）
angular.module('commonApp').run(['$rootScope', '$location', function($rootScope, $location) {
    $rootScope.appName = 'Common Test App';
    $rootScope.isLoggedIn = false;

    $rootScope.goTo = function(path) {
        $location.path(path);
    };

    $rootScope.$on('$routeChangeStart', function(event, next) {
        // route guard logic
    });
}]);

// ============================
// 11. $scope高度なパターン
// ============================

// 11.1 $scope.$watch
angular.module('commonApp').controller('WatchController', ['$scope', function($scope) {
    $scope.searchQuery = '';
    $scope.results = [];

    $scope.$watch('searchQuery', function(newVal, oldVal) {
        if (newVal !== oldVal) {
            $scope.search(newVal);
        }
    });

    $scope.$watchCollection('results', function(newCollection) {
        $scope.resultCount = newCollection.length;
    });

    $scope.search = function(query) {
        // search logic
    };
}]);

// 11.2 $scope.$on (event listener)
angular.module('commonApp').controller('EventController', ['$scope', '$rootScope', function($scope, $rootScope) {
    $scope.messages = [];

    $scope.$on('newMessage', function(event, data) {
        $scope.messages.push(data);
    });

    $scope.broadcast = function(msg) {
        $rootScope.$broadcast('newMessage', { text: msg, time: Date.now() });
    };

    $scope.emit = function(msg) {
        $scope.$emit('newMessage', { text: msg, time: Date.now() });
    };
}]);

// 11.3 $scope.$apply (manual digest)
angular.module('commonApp').controller('ApplyController', ['$scope', function($scope) {
    $scope.externalData = null;

    // 外部コールバックでの使用
    window.onExternalEvent = function(data) {
        $scope.$apply(function() {
            $scope.externalData = data;
        });
    };
}]);

// ============================
// 12. チェーン呼び出しパターン
// ============================

// 12.1 連続的なチェーン呼び出し
angular.module('chainApp', [])
    .constant('VERSION', '1.0')
    .value('settings', {})
    .service('ChainService', ['$http', function($http) {
        this.getData = function() {
            return $http.get('/api/data');
        };
    }])
    .controller('ChainController', ['$scope', 'ChainService', function($scope, ChainService) {
        $scope.data = null;
        $scope.load = function() {
            ChainService.getData().then(function(res) {
                $scope.data = res.data;
            });
        };
    }])
    .directive('chainDirective', [function() {
        return {
            restrict: 'A',
            link: function(scope, element) {
                element.addClass('chain-styled');
            }
        };
    }])
    .filter('chainFilter', [function() {
        return function(input) {
            return input ? input.toString().toUpperCase() : '';
        };
    }]);

// ============================
// 13. 高度なDIパターン
// ============================

// 13.1 var代入 + $inject
var AdvancedController = function($scope, $timeout, UserService, AuthService) {
    $scope.ready = false;
    $scope.init = function() {
        $scope.ready = true;
    };
};
AdvancedController.$inject = ['$scope', '$timeout', 'UserService', 'AuthService'];
angular.module('commonApp').controller('AdvancedController', AdvancedController);

// 13.2 const/let代入 + $inject (ES6)
const ModernController = function($scope, $http) {
    $scope.modern = true;
};
ModernController.$inject = ['$scope', '$http'];
angular.module('commonApp').controller('ModernController', ModernController);

// 13.3 IIFE (Immediately Invoked Function Expression) パターン
(function() {
    'use strict';

    angular.module('commonApp').controller('IIFEController', ['$scope', function($scope) {
        $scope.fromIIFE = true;
        $scope.greeting = 'Hello from IIFE';
    }]);
})();

// ============================
// 14. Decorator パターン
// ============================

// 14.1 $provide.decorator
angular.module('commonApp').config(['$provide', function($provide) {
    $provide.decorator('$log', ['$delegate', function($delegate) {
        var originalWarn = $delegate.warn;
        $delegate.warn = function() {
            // custom warning logic
            originalWarn.apply($delegate, arguments);
        };
        return $delegate;
    }]);
}]);

// 14.2 module.decorator (AngularJS 1.4+)
angular.module('commonApp').decorator('UserService', ['$delegate', '$log', function($delegate, $log) {
    var originalGetAll = $delegate.getAll;
    $delegate.getAll = function() {
        $log.debug('UserService.getAll called');
        return originalGetAll.apply($delegate, arguments);
    };
    return $delegate;
}]);

// ============================
// 15. resolve パターン（ルーティング）
// ============================

// This is typically in a .config block
// $routeProvider.when('/dashboard', {
//     templateUrl: 'views/dashboard.html',
//     controller: 'DashboardController',
//     resolve: {
//         userData: ['UserService', function(UserService) {
//             return UserService.getAll();
//         }],
//         settings: ['SettingsService', function(SettingsService) {
//             return SettingsService.load();
//         }]
//     }
// });

// ============================
// 16. $resource パターン (ngResource)
// ============================

// 16.1 基本的な$resource使用
angular.module('commonApp').factory('UserResource', ['$resource', function($resource) {
    return $resource('/api/users/:id', { id: '@id' }, {
        update: { method: 'PUT' },
        query: { method: 'GET', isArray: true }
    });
}]);

// ============================
// 17. Promise チェーンパターン
// ============================

angular.module('commonApp').service('AsyncService', ['$q', '$http', function($q, $http) {
    this.loadAll = function() {
        var deferred = $q.defer();
        $http.get('/api/data').then(
            function(response) { deferred.resolve(response.data); },
            function(error) { deferred.reject(error); }
        );
        return deferred.promise;
    };

    this.loadMultiple = function() {
        return $q.all([
            $http.get('/api/users'),
            $http.get('/api/settings')
        ]);
    };
}]);

// ============================
// 18. Nested Controller パターン
// ============================

angular.module('commonApp').controller('ParentController', ['$scope', function($scope) {
    $scope.parentData = 'from parent';
    $scope.sharedMethod = function() {
        return 'shared';
    };
}]);

angular.module('commonApp').controller('ChildController', ['$scope', function($scope) {
    // $scope.parentData は親コントローラーから継承
    $scope.childData = 'from child';
    $scope.childMethod = function() {
        return $scope.parentData + ' and child';
    };
}]);

// ============================
// 19. $scope on this代入パターン（混在）
// ============================

angular.module('commonApp').controller('MixedController', ['$scope', function($scope) {
    // $scope パターン
    $scope.scopeVar = 'via scope';

    // this パターン（controller as用）
    this.thisVar = 'via this';
    this.thisMethod = function() {
        return this.thisVar;
    };

    // 両方使用
    $scope.callThis = function() {
        return this.thisVar; // thisのコンテキスト問題
    };
}]);

// ============================
// 20. JSDoc コメント付きパターン
// ============================

/**
 * ユーザー管理サービス
 * @description ユーザーのCRUD操作を提供する
 * @param {Object} $http - HTTPサービス
 */
angular.module('commonApp').service('DocumentedService', ['$http', function($http) {
    /**
     * 全ユーザーを取得
     * @returns {Promise} ユーザー一覧
     */
    this.getAll = function() {
        return $http.get('/api/users');
    };
}]);

/**
 * ドキュメント付きコントローラー
 */
angular.module('commonApp').controller('DocumentedController', ['$scope', function($scope) {
    $scope.documented = true;
}]);
