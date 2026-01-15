// ES6 class構文のController/Service テスト

angular.module('testApp', []);

// パターン1: class参照
class UserController {
    constructor($scope, UserService) {
        $scope.users = [];
        $scope.loading = false;

        $scope.loadUsers = function() {
            $scope.loading = true;
            UserService.getUsers().then(function(users) {
                $scope.users = users;
                $scope.loading = false;
            });
        };
    }

    // classメソッド（controller as構文で使用可能）
    refresh() {
        console.log('refreshing...');
    }

    selectUser(user) {
        this.selectedUser = user;
    }
}

angular.module('testApp').controller('UserController', UserController);

// パターン2: class + $inject
class ProductController {
    constructor($scope, $http, ProductService) {
        this.products = [];

        ProductService.getProducts().then(function(data) {
            this.products = data;
        });
    }

    addProduct(product) {
        this.products.push(product);
    }
}
ProductController.$inject = ['$scope', '$http', 'ProductService'];

angular.module('testApp').controller('ProductController', ProductController);

// パターン3: class式直接
angular.module('testApp').controller('InlineController', class {
    constructor($scope) {
        $scope.message = 'Hello from inline class!';
        $scope.count = 0;

        $scope.increment = function() {
            $scope.count++;
        };
    }
});

// パターン4: DI配列内class
angular.module('testApp').controller('ArrayController', ['$scope', '$timeout', class {
    constructor($scope, $timeout) {
        $scope.status = 'ready';

        $timeout(function() {
            $scope.status = 'loaded';
        }, 1000);
    }
}]);

// class-based Service
class UserService {
    constructor($http, $q) {
        this.http = $http;
        this.q = $q;
        this.baseUrl = '/api/users';
    }

    getUsers() {
        return this.http.get(this.baseUrl).then(function(response) {
            return response.data;
        });
    }

    getUser(id) {
        return this.http.get(this.baseUrl + '/' + id);
    }

    createUser(userData) {
        return this.http.post(this.baseUrl, userData);
    }
}
UserService.$inject = ['$http', '$q'];

angular.module('testApp').service('UserService', UserService);

// class-based Factory
class ProductService {
    constructor($http) {
        this.http = $http;
    }

    getProducts() {
        return this.http.get('/api/products').then(function(res) {
            return res.data;
        });
    }

    getProduct(id) {
        return this.http.get('/api/products/' + id);
    }
}

angular.module('testApp').factory('ProductService', ProductService);

// ==========================================
// $rootScope テストケース
// ==========================================

// .run() での$rootScope プロパティ/メソッド定義
angular.module('testApp').run(['$rootScope', function($rootScope) {
    // $rootScope プロパティ定義
    $rootScope.appName = 'Test Application';
    $rootScope.currentUser = null;
    $rootScope.isLoggedIn = false;

    // $rootScope メソッド定義
    $rootScope.setCurrentUser = function(user) {
        $rootScope.currentUser = user;
        $rootScope.isLoggedIn = !!user;
    };

    $rootScope.logout = function() {
        $rootScope.currentUser = null;
        $rootScope.isLoggedIn = false;
    };

    // $rootScopeの参照
    console.log('App initialized:', $rootScope.appName);
}]);

// Controller内での$rootScope参照
class RootScopeTestController {
    constructor($scope, $rootScope) {
        // $rootScopeプロパティの参照
        $scope.appTitle = $rootScope.appName;
        $scope.user = $rootScope.currentUser;

        // $rootScopeメソッドの呼び出し
        $scope.login = function(userData) {
            $rootScope.setCurrentUser(userData);
        };

        $scope.doLogout = function() {
            $rootScope.logout();
        };

        // $rootScopeの別のプロパティ参照
        $scope.checkLogin = function() {
            return $rootScope.isLoggedIn;
        };
    }
}
RootScopeTestController.$inject = ['$scope', '$rootScope'];

angular.module('testApp').controller('RootScopeTestController', RootScopeTestController);

// 関数参照パターンでの$rootScope
function AppInitializer($rootScope, $timeout) {
    $rootScope.initTime = Date.now();
    $rootScope.showWelcome = true;

    $timeout(function() {
        $rootScope.showWelcome = false;
    }, 3000);
}
AppInitializer.$inject = ['$rootScope', '$timeout'];

angular.module('testApp').run(AppInitializer);
